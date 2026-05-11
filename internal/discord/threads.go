// Package discord — Z-3/Z-7 sessions-as-containers thread lifecycle.
//
// Z-7: a single hub channel (hub_channel_id) parents per-session
// threads. The bot spawns one thread per active session on
// hub_session_open and non-destructively archives it on close. No
// per-project channels — sessions for any project share the hub
// channel; the thread name (scope-slug) carries the disambiguation.
//
// Why threads over channels: Discord caps servers at 500 channels and
// imposes 5/5s channel-creation rate-limit server-wide. Threads have
// higher rate limits, leverage Discord's built-in archive (no manual
// export-and-delete), and preserve history on unarchive.
package discord

import (
	"fmt"

	"github.com/bwmarrin/discordgo"
)

// ThreadCreator + ThreadArchiver abstract Discord's thread API surface
// for testability. Production code injects *discordgo.Session; tests
// inject a mock.
type ThreadCreator interface {
	ThreadStartComplex(channelID string, data *discordgo.ThreadStart, options ...discordgo.RequestOption) (*discordgo.Channel, error)
}

type ThreadArchiver interface {
	ChannelEditComplex(channelID string, data *discordgo.ChannelEdit, options ...discordgo.RequestOption) (*discordgo.Channel, error)
}

// CreateSessionThread spawns a thread under the bot's hub channel for
// the named scope-slug. Returns the new thread's ID for storage in the
// session manifest's discord_thread_id frontmatter field.
//
// Z-7: parent channel = hub channel (resolveHubChannel). No
// per-project parent — the scope-slug thread name carries the
// disambiguation.
//
// Thread name = scope-slug (matches session-id slug portion for greppable
// matching between session manifests and Discord history).
//
// AutoArchiveDuration: 1 day (most session work fits within a day; user
// can extend by activity per Discord auto-archive semantics).
func (b *Bot) CreateSessionThread(scopeSlug string) (string, error) {
	if b.session == nil {
		return "", fmt.Errorf("discord session not started")
	}
	parent := b.resolveHubChannel()
	if parent == "" {
		return "", fmt.Errorf("no hub channel configured for thread parent")
	}
	return createSessionThread(b.session, parent, scopeSlug)
}

func createSessionThread(s ThreadCreator, projectChannelID, scopeSlug string) (string, error) {
	if projectChannelID == "" {
		return "", fmt.Errorf("project channel ID required")
	}
	if scopeSlug == "" {
		return "", fmt.Errorf("scope slug required")
	}
	thread, err := s.ThreadStartComplex(projectChannelID, &discordgo.ThreadStart{
		Name:                scopeSlug,
		Type:                discordgo.ChannelTypeGuildPublicThread,
		AutoArchiveDuration: 1440, // 24h in minutes
		Invitable:           false,
	})
	if err != nil {
		return "", fmt.Errorf("thread start: %w", err)
	}
	return thread.ID, nil
}

// ArchiveSessionThread non-destructively archives the given thread. Per
// Discord semantics, archived threads remain visible (collapsed) and can
// be unarchived if the session resumes. Pinned messages survive.
func (b *Bot) ArchiveSessionThread(threadID string) error {
	if b.session == nil {
		return fmt.Errorf("discord session not started")
	}
	return archiveSessionThread(b.session, threadID)
}

func archiveSessionThread(s ThreadArchiver, threadID string) error {
	if threadID == "" {
		return fmt.Errorf("thread ID required")
	}
	archived := true
	_, err := s.ChannelEditComplex(threadID, &discordgo.ChannelEdit{
		Archived: &archived,
	})
	if err != nil {
		return fmt.Errorf("channel edit (archive): %w", err)
	}
	return nil
}
