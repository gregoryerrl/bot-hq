package ui

import (
	"fmt"
	"sort"
	"strings"
	"time"

	"github.com/charmbracelet/lipgloss"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// sessionStripPreviewLines is the number of recent message previews
// shown per session card. Z-8e: user chose "ID + last 2-3 message
// previews" — three lines balances signal vs vertical density.
const sessionStripPreviewLines = 3

// sessionStripMaxContentChars is the per-line content truncation limit
// inside each card. Keeps cards predictable-width regardless of
// terminal width; long lines get an ellipsis.
const sessionStripMaxContentChars = 28

// renderSessionStrips renders the right-pane scrollable strip column
// for the Hub tab (Z-8e). Each active session gets a card containing
// session-id, relative-timestamp of last activity, and the last
// sessionStripPreviewLines message previews.
//
// sessions is the live list of active sessions. messages is the full
// message stream the HubTab has seen; we filter it per-session here
// to extract previews. width is the column's render width. height is
// the viewport height available for the column.
//
// Returns a vertical string suitable for lipgloss horizontal-join with
// the chat viewport.
func renderSessionStrips(sessions []protocol.Session, messages []protocol.Message, width, height int) string {
	// Z-8 followup: active-only filter — matches Sessions tab default.
	// Done/paused sessions belong in retrospective queries, not the
	// live strip.
	active := make([]protocol.Session, 0, len(sessions))
	for _, sess := range sessions {
		if sess.Status == protocol.SessionActive {
			active = append(active, sess)
		}
	}
	if len(active) == 0 {
		emptyStyle := lipgloss.NewStyle().Foreground(ColorStatus).Width(width)
		return emptyStyle.Render("No active sessions.\nOpen one via\nhub_session_open.")
	}

	// Bucket messages by session_id (only session-scoped rows).
	bySession := make(map[string][]protocol.Message)
	for _, m := range messages {
		if m.SessionID == "" {
			continue
		}
		bySession[m.SessionID] = append(bySession[m.SessionID], m)
	}

	// Sort sessions by most-recent-activity first so the user's eye
	// hits the busiest sessions at the top.
	sort.SliceStable(active, func(i, j int) bool {
		iMsgs := bySession[active[i].ID]
		jMsgs := bySession[active[j].ID]
		var iLast, jLast time.Time
		if len(iMsgs) > 0 {
			iLast = iMsgs[len(iMsgs)-1].Created
		}
		if len(jMsgs) > 0 {
			jLast = jMsgs[len(jMsgs)-1].Created
		}
		return iLast.After(jLast)
	})

	var cards []string
	for _, sess := range active {
		cards = append(cards, renderSessionCard(sess, bySession[sess.ID], width))
	}
	return strings.Join(cards, "\n")
}

// renderSessionCard builds a single session card: id, ts, recent previews.
func renderSessionCard(sess protocol.Session, sessionMsgs []protocol.Message, width int) string {
	idStyle := lipgloss.NewStyle().Foreground(ColorSession).Bold(true)
	tsStyle := lipgloss.NewStyle().Foreground(ColorStatus)
	previewStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("#CCCCCC"))

	// Header line: id (truncated to fit width) + relative timestamp.
	shortID := sess.ID
	if len(shortID) > width-12 && width > 14 {
		shortID = shortID[:width-12] + "…"
	}
	var lastTS time.Time
	if len(sessionMsgs) > 0 {
		lastTS = sessionMsgs[len(sessionMsgs)-1].Created
	}
	relTS := relativeTimestamp(lastTS)
	header := fmt.Sprintf("%s %s",
		idStyle.Render(shortID),
		tsStyle.Render(relTS),
	)

	// Pick the last sessionStripPreviewLines messages.
	previews := sessionMsgs
	if len(previews) > sessionStripPreviewLines {
		previews = previews[len(previews)-sessionStripPreviewLines:]
	}
	var previewLines []string
	for _, m := range previews {
		line := fmt.Sprintf("%s: %s", m.FromAgent, truncate(strings.ReplaceAll(m.Content, "\n", " "), sessionStripMaxContentChars))
		previewLines = append(previewLines, previewStyle.Render(line))
	}
	if len(previewLines) == 0 {
		previewLines = append(previewLines, previewStyle.Render("(no activity)"))
	}

	cardStyle := lipgloss.NewStyle().
		Border(lipgloss.NormalBorder(), true, false, false, false).
		BorderForeground(lipgloss.Color("#555555")).
		Width(width)

	body := strings.Join(append([]string{header}, previewLines...), "\n")
	return cardStyle.Render(body)
}

// relativeTimestamp renders a Created time as "Xs ago" / "Xm ago" / "Xh ago"
// for compact card headers. Empty/zero time yields "—".
func relativeTimestamp(t time.Time) string {
	if t.IsZero() {
		return "—"
	}
	elapsed := time.Since(t)
	switch {
	case elapsed < time.Minute:
		return fmt.Sprintf("%ds ago", int(elapsed.Seconds()))
	case elapsed < time.Hour:
		return fmt.Sprintf("%dm ago", int(elapsed.Minutes()))
	case elapsed < 24*time.Hour:
		return fmt.Sprintf("%dh ago", int(elapsed.Hours()))
	default:
		return fmt.Sprintf("%dd ago", int(elapsed.Hours()/24))
	}
}

// truncate cuts a string at maxLen with an ellipsis. Pure helper.
func truncate(s string, maxLen int) string {
	if maxLen <= 1 {
		return s
	}
	if len(s) <= maxLen {
		return s
	}
	return s[:maxLen-1] + "…"
}
