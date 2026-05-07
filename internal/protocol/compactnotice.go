package protocol

// Phase S S-2 autocompact-detect+broadcast helpers.
//
// PreCompact hook (Claude Code) fires a Bash command emitting
// hub_send with MsgCompactNotice + agent-id payload. Discriminator-
// broadcast: [HR]-tagged when peer has active fire-in-flight
// (current_task non-empty per Phase-R-followup-1 (f) data-model);
// untagged when both peers idle.
//
// Symmetric MsgResume signals post-compact context-reloaded so peers
// can resume cross-talk safely; resume is informational (always
// untagged compact-class).
//
// Brian's S-5 message-buffer flush-pre-compact interaction is wired
// via daemon-side handler on MsgCompactNotice arrival from
// FromAgent="brian": daemon calls Brian.FlushPendingBatch so context-
// preserve includes pending msgs pre-compact.

import (
	"fmt"
)

// AgentActiveFireInFlight reports whether an agent has a non-empty
// current_task field set, indicating active fire-in-flight that peers
// should be aware of. Used by the compact-notice discriminator to
// decide [HR]-tag application: any peer with non-empty current_task
// → [HR]; all peers idle (current_task empty) → untagged.
func AgentActiveFireInFlight(agent Agent) bool {
	return agent.CurrentTask != ""
}

// AnyPeerActiveFireInFlight reports whether ANY peer (other than self)
// has active fire-in-flight per AgentActiveFireInFlight. Returns true
// if at least one non-self agent has CurrentTask set.
func AnyPeerActiveFireInFlight(selfAgentID string, agents []Agent) bool {
	for _, a := range agents {
		if a.ID == selfAgentID {
			continue
		}
		if AgentActiveFireInFlight(a) {
			return true
		}
	}
	return false
}

// BuildCompactNoticeContent formats the MsgCompactNotice content
// payload with discriminator-driven [HR] tag. agentID is the
// compacting agent's id; peerActiveFireInFlight controls [HR] gating.
//
// Format:
//
//	With peer-active:    "[HR] <agent-id>|compacting"
//	With both-idle:      "<agent-id>|compacting"
//
// The compact-pipe format pairs with MsgUpdate-class compact emits;
// [HR] prefix applies only when peer-cross-coord-relevant.
func BuildCompactNoticeContent(agentID string, peerActiveFireInFlight bool) string {
	if peerActiveFireInFlight {
		return fmt.Sprintf("[HR] %s|compacting", agentID)
	}
	return fmt.Sprintf("%s|compacting", agentID)
}

// BuildResumeContent formats the MsgResume content payload. Always
// untagged compact-class (informational; resume is not user-
// attention-warranting per S-2 design).
func BuildResumeContent(agentID string) string {
	return fmt.Sprintf("%s|resumed-context-reloaded", agentID)
}
