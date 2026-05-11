package discord

import (
	"fmt"
	"log"
	"strings"
	"sync"
	"time"

	"github.com/bwmarrin/discordgo"
	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// Bot bridges Discord messages to and from the hub.
//
// Routing (Phase R R4 + Z-7): MsgFlag → flagsChannelID (or hub
// fallback); session-scoped messages (msg.SessionID != "") → that
// session's thread under hubChannelID (or hub fallback if no thread
// registered); everything else → hubChannelID (or legacy channelID).
type Bot struct {
	session   *discordgo.Session
	hub       *hub.Hub
	channelID string // legacy single-channel (back-compat fallback)
	// Phase R R4 multi-channel routing
	hubChannelID   string // primary post-R4 channel for hub messages
	flagsChannelID string // MsgFlag class destination
	botUserID      string
	mu             sync.Mutex
	stopCh         chan struct{}

	// Z-7: session_id↔thread_id registry. Populated by session-open
	// hook + bootstrapThreadRegistry at Start. Outbound messages with
	// SessionID look up their thread here; inbound messages from a
	// known thread auto-tag with the session_id.
	threadsMu       sync.RWMutex
	threadByID      map[string]string // session_id → thread_id
	sessionByThread map[string]string // thread_id → session_id
}

// NewBot creates a new Discord bot. It validates the token and channel
// configuration but does not open the Discord session until Start is called.
//
// At least one of channelID OR hubChannelID must be populated (legacy
// single-channel OR R4-era hub-channel mode). flagsChannelID is
// optional — empty falls back to the hub channel.
func NewBot(token, channelID, hubChannelID, flagsChannelID string, h *hub.Hub) (*Bot, error) {
	if token == "" {
		return nil, fmt.Errorf("discord bot token is required")
	}
	if channelID == "" && hubChannelID == "" {
		return nil, fmt.Errorf("discord channel_id (legacy) OR hub_channel_id (Phase R R4) is required")
	}
	if h == nil {
		return nil, fmt.Errorf("hub is required")
	}

	sess, err := discordgo.New("Bot " + token)
	if err != nil {
		return nil, fmt.Errorf("create discord session: %w", err)
	}

	// Only subscribe to guild message events
	sess.Identify.Intents = discordgo.IntentsGuildMessages

	return &Bot{
		session:        sess,
		hub:            h,
		channelID:      channelID,
		hubChannelID:   hubChannelID,
		flagsChannelID: flagsChannelID,
		stopCh:         make(chan struct{}),
	}, nil
}

// resolveHubChannel returns the effective primary channel for hub-class
// messages. Prefers hubChannelID (Phase R R4); falls back to legacy
// channelID. Pure helper — no I/O.
func (b *Bot) resolveHubChannel() string {
	if b.hubChannelID != "" {
		return b.hubChannelID
	}
	return b.channelID
}

// channelForMessage returns the destination channel ID for a hub message.
//
// Routing matrix (first match wins):
//  1. MsgFlag → flagsChannelID; falls back to hub-channel if empty
//     (flags are global signals, kept in the hub channel — not in a
//     session's thread — so they surface across all session views).
//  2. msg.SessionID != "" with a registered thread → that thread.
//     Session-scoped chatter lands in the session's thread; an
//     unknown session-id falls through to the hub channel (pre-Z-7
//     session OR thread-create failed at open time).
//  3. everything else → hub-channel (hubChannelID OR legacy channelID).
//
// Pre-R4 single-channel deployments (legacy channelID set, R4 fields
// empty) still route everything to channelID via the hub fallback.
func (b *Bot) channelForMessage(msg protocol.Message) string {
	hub := b.resolveHubChannel()
	if msg.Type == protocol.MsgFlag {
		if b.flagsChannelID != "" {
			return b.flagsChannelID
		}
		return hub
	}
	if msg.SessionID != "" {
		if tid := b.threadForSession(msg.SessionID); tid != "" {
			return tid
		}
	}
	return hub
}

// listensOnChannel reports whether the bot's incoming-message filter
// should accept messages from the given channel ID. Accepts any
// configured top-level channel ({channelID, hubChannelID,
// flagsChannelID}) plus any registered session-thread.
func (b *Bot) listensOnChannel(id string) bool {
	if id == "" {
		return false
	}
	if id == b.channelID && b.channelID != "" {
		return true
	}
	if id == b.hubChannelID && b.hubChannelID != "" {
		return true
	}
	if id == b.flagsChannelID && b.flagsChannelID != "" {
		return true
	}
	// Z-7: session-threads registered at open / discovered at startup.
	if b.sessionForThread(id) != "" {
		return true
	}
	return false
}

// Start opens the Discord websocket connection, registers the bot as
// a "discord" agent on the hub, and begins forwarding messages in both
// directions.
func (b *Bot) Start() error {
	// Register the message handler before opening so we don't miss events
	b.session.AddHandler(b.handleDiscordMessage)

	if err := b.session.Open(); err != nil {
		return fmt.Errorf("open discord session: %w", err)
	}

	// Store our own user ID so we can ignore our own messages
	b.mu.Lock()
	b.botUserID = b.session.State.User.ID
	b.mu.Unlock()

	// Register as a "discord" agent in the hub DB
	if err := b.hub.DB.RegisterAgent(protocol.Agent{
		ID:         "discord",
		Name:       "Discord",
		Type:       protocol.AgentDiscord,
		Status:     protocol.StatusOnline,
		Registered: time.Now(),
		LastSeen:   time.Now(),
	}); err != nil {
		log.Printf("[discord] failed to register agent: %v", err)
	}

	// Z-7: load session_id↔thread_id mappings from existing active
	// session manifests so outbound routing + inbound tagging survive
	// daemon restarts.
	b.bootstrapThreadRegistry()

	// Subscribe to hub messages targeted at the "discord" agent
	ch := b.hub.RegisterWSClient("discord")
	go b.forwardToDiscord(ch)

	log.Printf("[discord] bot started, listening on channel %s", b.channelID)
	return nil
}

// Stop unregisters the bot from the hub and closes the Discord session.
func (b *Bot) Stop() error {
	// Signal the forwarder goroutine to stop
	close(b.stopCh)

	// Unregister from hub
	b.hub.UnregisterWSClient("discord")

	// Mark agent as offline
	if err := b.hub.DB.UpdateAgentStatus("discord", protocol.StatusOffline); err != nil {
		log.Printf("[discord] failed to update agent status: %v", err)
	}

	// Close Discord session
	if b.session != nil {
		return b.session.Close()
	}
	return nil
}

// handleDiscordMessage is the discordgo event handler for incoming messages.
// It filters to the configured channel, ignores its own messages, and
// inserts the message into the hub DB.
func (b *Bot) handleDiscordMessage(s *discordgo.Session, m *discordgo.MessageCreate) {
	// Ignore messages from channels outside our configured set.
	// Phase R R4: filter against {channelID, hub/flags/sessions IDs}.
	if !b.listensOnChannel(m.ChannelID) {
		return
	}

	// Ignore our own messages to prevent loops
	b.mu.Lock()
	selfID := b.botUserID
	b.mu.Unlock()
	if m.Author.ID == selfID {
		return
	}

	// Ignore empty messages
	content := m.Content
	if content == "" {
		return
	}

	// Format: include the Discord username for context
	text := fmt.Sprintf("[%s] %s", m.Author.Username, content)

	// Z-7: if the message came from a registered session-thread, tag
	// the hub-row with that session_id so it surfaces in the right
	// session stream (and not the global / cross-session feed).
	sessionID := b.sessionForThread(m.ChannelID)

	// Insert into hub DB as a message from the discord agent
	if _, err := b.hub.DB.InsertMessage(protocol.Message{
		SessionID: sessionID,
		FromAgent: "discord",
		Type:      protocol.MsgResponse,
		Content:   text,
		Created:   time.Now(),
	}); err != nil {
		log.Printf("[discord] failed to insert message: %v", err)
	}
}

// forwardToDiscord reads messages from the hub's WS client channel and
// sends any messages addressed to the "discord" agent to the Discord channel.
func (b *Bot) forwardToDiscord(ch <-chan protocol.Message) {
	var lastContent string
	var lastSent time.Time

	for {
		select {
		case <-b.stopCh:
			return
		case msg, ok := <-ch:
			if !ok {
				return
			}

			// Don't echo back our own messages
			if msg.FromAgent == "discord" {
				continue
			}

			if !shouldForwardToDiscord(msg) {
				continue
			}

			// Deduplicate: skip if same content was sent within 5 seconds
			if msg.Content == lastContent && time.Since(lastSent) < 5*time.Second {
				continue
			}
			lastContent = msg.Content
			lastSent = time.Now()

			// Phase R R4: route to per-class channel. Pre-R4 single-channel
			// deployments still route to channelID via channelForMessage's
			// fallback chain.
			dest := b.channelForMessage(msg)

			// Phase-R-followup: render via Discord embed (boxed message)
			// per user msg post-R4b. buildEmbedChunks returns 1+ embeds
			// covering content >4096 chars (embed description limit).
			// First embed carries title/author/footer; continuation embeds
			// are description-only with the same color sidebar.
			embeds := buildEmbedChunks(msg)
			for _, embed := range embeds {
				if _, err := b.session.ChannelMessageSendEmbed(dest, embed); err != nil {
					log.Printf("[discord] failed to send embed: %v", err)
				}
			}
		}
	}
}

// Phase-R-followup embed-mode color palette (Discord brand colors).
// Red shared between Flag + Error per Refine-4 — title text + emoji
// disambiguates for color-blind accessibility.
const (
	colorFlagRed         = 0xed4245 // Discord danger
	colorErrorRed        = 0xed4245 // same as flag; disambiguated by title
	colorHRPink          = 0xeb459e // high-relevance duo-consensus
	colorResultGreen     = 0x57f287 // Discord success
	colorCommandYellow   = 0xfee75c // Discord warning
	colorResponseBlurple = 0x5865f2 // Discord brand
	colorUpdateGray      = 0x99aab5 // Discord muted
)

// embedDescriptionLimit is the Discord embed description char limit.
// Content longer than this is split into multiple embeds per
// buildEmbedChunks; first embed carries title/author/footer while
// continuation embeds are description-only with same color sidebar.
const embedDescriptionLimit = 4096

// buildEmbed returns a single Discord embed for a hub message, applying
// per-class color + title + author + footer per Phase-R-followup
// design. Class-precedence (first-match-wins) per Rain msg 15589
// Refine-1: MsgFlag → MsgError → [HR] prefix → MsgResult → MsgCommand
// → MsgResponse → MsgUpdate (default).
//
// Author=nil on Flag and [HR] classes per Phase R R2 authorless-display
// continuity (sender stripped at render; DB from_agent preserved for
// forensics).
//
// Z-5i: author renders as `<from>` only (the PM-class arrow rendering
// was display-only inertia from pre-Phase-S-4 PM semantics).
//
// Description carries content unchunked here; buildEmbedChunks handles
// >4096-char splits. Caller pairs with ChannelMessageSendEmbed.
func buildEmbed(msg protocol.Message) *discordgo.MessageEmbed {
	hasHR := strings.HasPrefix(msg.Content, "[HR] ") || strings.HasPrefix(msg.Content, "[HR]\n")
	embed := &discordgo.MessageEmbed{
		Description: msg.Content,
	}
	if !msg.Created.IsZero() {
		embed.Timestamp = msg.Created.Format(time.RFC3339)
	}
	if msg.ID > 0 {
		embed.Footer = &discordgo.MessageEmbedFooter{Text: fmt.Sprintf("msg %d", msg.ID)}
	}

	switch {
	case msg.Type == protocol.MsgFlag:
		embed.Color = colorFlagRed
		embed.Title = "🚨 ATTENTION NEEDED"
		// Author intentionally nil per R2 authorless-display.
	case msg.Type == protocol.MsgError:
		embed.Color = colorErrorRed
		embed.Title = "Error"
		embed.Author = authorFor(msg)
	case hasHR:
		embed.Color = colorHRPink
		embed.Title = "[HR]"
		// Author intentionally nil per R2 authorless-display.
	case msg.Type == protocol.MsgResult:
		embed.Color = colorResultGreen
		embed.Title = "Result"
		embed.Author = authorFor(msg)
	case msg.Type == protocol.MsgCommand:
		embed.Color = colorCommandYellow
		embed.Title = "Command"
		embed.Author = authorFor(msg)
	case msg.Type == protocol.MsgResponse:
		embed.Color = colorResponseBlurple
		embed.Author = authorFor(msg)
	default: // MsgUpdate or unrecognized type
		embed.Color = colorUpdateGray
		embed.Author = authorFor(msg)
	}
	return embed
}

// authorFor returns the embed Author = the sender's name.
// Z-5i: dropped the "<from> → <to>" PM-style rendering — Phase S S-4
// removed PM semantics; the arrow was display-only inertia that lied
// about the wire shape. ToAgent is still a routing field (used by the
// Discord forwarder filter via shouldForwardToDiscord) but it doesn't
// surface in the user-visible embed.
func authorFor(msg protocol.Message) *discordgo.MessageEmbedAuthor {
	return &discordgo.MessageEmbedAuthor{Name: msg.FromAgent}
}

// buildEmbedChunks returns 1+ embeds covering content longer than
// embedDescriptionLimit (4096). First embed carries title/author/footer
// with first chunk; continuation embeds are description-only with the
// same color sidebar (visual continuity).
//
// 99% of hub messages fit in a single embed; chunking is the safety
// path for unusually long content (verbose BRAIN-cycle drafts /
// long status reports).
func buildEmbedChunks(msg protocol.Message) []*discordgo.MessageEmbed {
	full := buildEmbed(msg)
	if len(msg.Content) <= embedDescriptionLimit {
		return []*discordgo.MessageEmbed{full}
	}
	chunks := splitMessage(msg.Content, embedDescriptionLimit)
	embeds := make([]*discordgo.MessageEmbed, 0, len(chunks))
	full.Description = chunks[0]
	embeds = append(embeds, full)
	for _, c := range chunks[1:] {
		embeds = append(embeds, &discordgo.MessageEmbed{
			Description: c,
			Color:       full.Color,
		})
	}
	return embeds
}

// splitMessage splits a message into chunks that fit within Discord's
// character limit, preferring to break at newlines.
func splitMessage(text string, limit int) []string {
	if len(text) <= limit {
		return []string{text}
	}

	var chunks []string
	remaining := text
	for len(remaining) > 0 {
		if len(remaining) <= limit {
			chunks = append(chunks, remaining)
			break
		}

		// Try to split at a newline
		splitAt := -1
		for i := limit; i >= limit/2; i-- {
			if remaining[i] == '\n' {
				splitAt = i
				break
			}
		}
		if splitAt == -1 {
			splitAt = limit
		}

		chunks = append(chunks, remaining[:splitAt])
		remaining = remaining[splitAt:]
		// Trim leading whitespace from remainder
		for len(remaining) > 0 && (remaining[0] == ' ' || remaining[0] == '\n' || remaining[0] == '\t') {
			remaining = remaining[1:]
		}
	}

	return chunks
}

// shouldForwardToDiscord decides whether a hub message should bridge through
// to the Discord channel. Bug #1 fix: forward broadcasts (ToAgent=="") in
// addition to discord-routed and flag messages. The original filter
// (ToAgent != "discord" && Type != MsgFlag) silently dropped every broadcast,
// hiding ~half of agent activity from Discord. Audience-driven routing per
// DISC v2 (msg 2147 lock + Worktree C const extraction in commit 2a/2b).
//
// Forward when ANY of:
//   - ToAgent == "discord" (explicitly addressed)
//   - ToAgent == "" (broadcast — user is one of the intended audiences)
//   - Type == MsgFlag (attention notification regardless of routing)
//
// Skip peer-routed PMs (ToAgent in {"brian", "rain", ...}). Discord's
// audience is the user; peer coordination should not surface as Discord noise.
func shouldForwardToDiscord(msg protocol.Message) bool {
	if msg.Type == protocol.MsgFlag {
		return true
	}
	return msg.ToAgent == "discord" || msg.ToAgent == ""
}
