package protocol

import "strings"

// IsElevated reports whether a hub message is a cross-session signal
// that should surface in the main hub view even when the row is
// session-scoped.
//
// Elevation classes (Z-8c):
//   - MsgFlag — attention class (force-push request, user-blocking
//     halt, plan-cap critical, etc.).
//   - content prefixed with "[HR] " — high-relevance duo-consensus
//     elevation per Phase R R2; Rain emits these from within sessions
//     when a signal needs user/Emma attention beyond the session.
//
// Used by:
//   - Hub tab main-chat filter (left pane, Z-8e)
//   - Emma's pollLoop (Z-8d) — Emma sees broadcasts + elevations only.
//
// Pure helper; no I/O. Callers pre-filter by Type/Content cheaply.
func IsElevated(msg Message) bool {
	if msg.Type == MsgFlag {
		return true
	}
	if strings.HasPrefix(msg.Content, "[HR] ") || strings.HasPrefix(msg.Content, "[HR]\n") {
		return true
	}
	return false
}

// PassesMainHubView reports whether a hub message should surface in
// the Hub tab's main-chat pane (left side, Z-8e) and to Emma's
// pollLoop (Z-8d).
//
// Rule: SessionID == "" (broadcast / main-hub / system) OR elevated.
// Session-scoped non-elevated chatter is invisible to the main view
// and to Emma — sessions own their own conversation.
//
// One DB row, two views: a [HR] row from session X has SessionID=X
// (truth about origin); the container shows it (SessionID filter)
// AND main hub shows it (elevation gate). No duplication.
func PassesMainHubView(msg Message) bool {
	if msg.SessionID == "" {
		return true
	}
	return IsElevated(msg)
}
