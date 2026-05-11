package protocol

// FilterForSession reports whether a message should be visible to a
// poller that holds mySessionID as its own BOT_HQ_SESSION_ID. Z-5d:
// single chokepoint replacing the inline filter that Z-3d-fix5 added
// in two places (brian + rain pollLoops). Outbound auto-tagging in
// hub_send is a separate concern and stays in tools_message.go.
//
// Rules:
//
//   - mySessionID == "" means the poller is out-of-session (clive,
//     discord-relay, the daemon's own observers). It sees everything.
//
//   - Otherwise, the poller sees messages tagged to its own session
//     OR untagged broadcasts (msg.SessionID == "" — system events,
//     main-hub user messages, emma announcements that aren't
//     scoped). Other session_ids are filtered out.
//
// Returns true when the message should be processed; false to drop.
func FilterForSession(msg Message, mySessionID string) bool {
	if mySessionID == "" {
		return true
	}
	if msg.SessionID == "" {
		return true
	}
	return msg.SessionID == mySessionID
}
