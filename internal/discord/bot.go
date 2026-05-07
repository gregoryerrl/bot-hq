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
// Phase R R4 multi-channel routing: when hubChannelID is populated,
// hub messages route by class — MsgFlag → flagsChannelID; session-event
// class (content prefix `[SESSION:` per R5 forward-compat) → sessionsChannelID;
// everything else → hubChannelID. Empty class-specific IDs fall back to
// hubChannelID. Empty hubChannelID falls back to legacy channelID.
type Bot struct {
	session   *discordgo.Session
	hub       *hub.Hub
	channelID string // legacy single-channel (back-compat fallback)
	// Phase R R4 multi-channel routing
	hubChannelID      string // primary post-R4 channel for hub messages
	flagsChannelID    string // MsgFlag class destination
	sessionsChannelID string // session-event class destination ([SESSION: prefix)
	botUserID         string
	mu                sync.Mutex
	stopCh            chan struct{}
}

// NewBot creates a new Discord bot. It validates the token and channel
// configuration but does not open the Discord session until Start is called.
//
// Phase R R4 multi-channel: at least one of channelID OR hubChannelID
// must be populated (legacy single-channel OR R4 multi-channel mode).
// flagsChannelID + sessionsChannelID are optional — empty values fall
// back to hubChannelID per partial-migration semantics.
func NewBot(token, channelID, hubChannelID, flagsChannelID, sessionsChannelID string, h *hub.Hub) (*Bot, error) {
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
		session:           sess,
		hub:               h,
		channelID:         channelID,
		hubChannelID:      hubChannelID,
		flagsChannelID:    flagsChannelID,
		sessionsChannelID: sessionsChannelID,
		stopCh:            make(chan struct{}),
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

// channelForMessage returns the destination channel ID for a hub message
// per Phase R R4 routing discriminator.
//
// Routing matrix:
//   - MsgFlag → flagsChannelID; falls back to hub-channel if empty (partial-migration)
//   - content prefix "[SESSION:" → sessionsChannelID; falls back to hub-channel if empty
//   - everything else → hub-channel (hubChannelID OR legacy channelID)
//
// Pre-R4 single-channel deployments (legacy channelID set, R4 fields empty)
// route everything to channelID.
func (b *Bot) channelForMessage(msg protocol.Message) string {
	hub := b.resolveHubChannel()
	if msg.Type == protocol.MsgFlag {
		if b.flagsChannelID != "" {
			return b.flagsChannelID
		}
		return hub
	}
	// Session-event detection via [SESSION:<8>] content prefix per Phase R R5
	// forward-compat. Phase R R5 will codify the prefix on session-create /
	// session-close emits; until then this branch matches no current emits
	// (defensive forward-compat — no behavior change pre-R5).
	if strings.HasPrefix(msg.Content, "[SESSION:") {
		if b.sessionsChannelID != "" {
			return b.sessionsChannelID
		}
		return hub
	}
	return hub
}

// listensOnChannel reports whether the bot's incoming-message filter
// should accept messages from the given channel ID. Phase R R4 expands
// from single-channel to {channelID, hubChannelID, flagsChannelID,
// sessionsChannelID} — any non-empty channel-ID value the bot is
// configured for. Pre-R4 single-channel deployments accept only
// channelID. Pure helper — no I/O.
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
	if id == b.sessionsChannelID && b.sessionsChannelID != "" {
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

	// Insert into hub DB as a message from the discord agent
	if _, err := b.hub.DB.InsertMessage(protocol.Message{
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

			// Format and send to Discord. Phase R R2: [HR]-tagged content +
			// MsgFlag class display-strip the sender attribution per Rain
			// msg 15510 BRAIN-final + msg 15545 Refine. DB preserves
			// from_agent for forensics per `from_agent` column.
			var text string
			if msg.Type == protocol.MsgFlag {
				text = fmt.Sprintf("🚨 **ATTENTION NEEDED**:\n%s", msg.Content)
			} else if strings.HasPrefix(msg.Content, "[HR] ") || strings.HasPrefix(msg.Content, "[HR]\n") {
				text = msg.Content
			} else {
				text = fmt.Sprintf("**[%s]** %s", msg.FromAgent, msg.Content)
			}

			// Phase R R4: route to per-class channel. Pre-R4 single-channel
			// deployments still route to channelID via channelForMessage's
			// fallback chain.
			dest := b.channelForMessage(msg)

			// Discord has a 2000 character limit; split if needed
			chunks := splitMessage(text, 2000)
			for _, chunk := range chunks {
				if _, err := b.session.ChannelMessageSend(dest, chunk); err != nil {
					log.Printf("[discord] failed to send message: %v", err)
				}
			}
		}
	}
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
