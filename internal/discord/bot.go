package discord

import (
	"fmt"
	"log"
	"sync"
	"time"

	"github.com/bwmarrin/discordgo"
	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// Bot bridges Discord messages to and from the hub.
// It listens on a single configured channel, forwarding incoming
// Discord messages into the hub and sending hub messages addressed
// to the "discord" agent back to the Discord channel.
type Bot struct {
	session   *discordgo.Session
	hub       *hub.Hub
	channelID string
	botUserID string
	mu        sync.Mutex
	stopCh    chan struct{}
}

// NewBot creates a new Discord bot. It validates the token and channelID
// but does not open the Discord session until Start is called.
func NewBot(token, channelID string, h *hub.Hub) (*Bot, error) {
	if token == "" {
		return nil, fmt.Errorf("discord bot token is required")
	}
	if channelID == "" {
		return nil, fmt.Errorf("discord channel ID is required")
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
		session:   sess,
		hub:       h,
		channelID: channelID,
		stopCh:    make(chan struct{}),
	}, nil
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
	// Ignore messages from other channels
	if m.ChannelID != b.channelID {
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

			// Format and send to Discord
			text := fmt.Sprintf("**[%s]** %s", msg.FromAgent, msg.Content)

			// Discord has a 2000 character limit; split if needed
			chunks := splitMessage(text, 2000)
			for _, chunk := range chunks {
				if _, err := b.session.ChannelMessageSend(b.channelID, chunk); err != nil {
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
