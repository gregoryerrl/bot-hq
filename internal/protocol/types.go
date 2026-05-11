package protocol

import "time"

type AgentType string

const (
	AgentCoder   AgentType = "coder"
	AgentVoice   AgentType = "voice"
	AgentBrian   AgentType = "brian"
	AgentDiscord AgentType = "discord"
	AgentQA      AgentType = "qa"
	AgentEmma    AgentType = "emma"
)

func (a AgentType) Valid() bool {
	switch a {
	case AgentCoder, AgentVoice, AgentBrian, AgentDiscord, AgentQA, AgentEmma:
		return true
	}
	return false
}

type AgentStatus string

const (
	StatusOnline  AgentStatus = "online"
	StatusWorking AgentStatus = "working"
	StatusOffline AgentStatus = "offline"
)

func (a AgentStatus) Valid() bool {
	switch a {
	case StatusOnline, StatusWorking, StatusOffline:
		return true
	}
	return false
}

// MessageType classifies a hub message's intent (separate from
// hub.go routing — only MsgFlag is hub-special-cased; other types are
// semantic for agent-receivers + audit replay).
//
// Phase J T1.7 (B7) — taxonomy codified per
// docs/plans/2026-04-29-msg-type-taxonomy-audit.md §4. Active types
// (6): Update/Response/Command/Result/Error/Flag. Deprecated/legacy
// (2): Handshake/Question (kept Valid() to avoid breaking existing
// hub history; duo agents avoid in new emits).
//
// Cross-mapping with R9 AUDIENCE-CLASS-DISCRIMINATOR ([HR] tag) and
// R8 COMPACT-COMMIT-FORMAT (compact-eligible) per audit §4.3:
//
//	Type        | [HR] default | compact-eligible? | hub special
//	------------|--------------|-------------------|------------
//	MsgFlag     | always       | no (verbose)      | YES (alerts)
//	MsgError    | always       | no                | no
//	MsgCommand  | usually      | no                | no
//	MsgResult   | sometimes    | yes (untagged)    | no
//	MsgUpdate   | rarely       | yes (untagged)    | no
//	MsgResponse | varies       | depends           | no
//	MsgHandshake| n/a          | yes (".")         | no   [DEPRECATED]
//	MsgQuestion | n/a          | n/a               | no   [DEPRECATED]
type MessageType string

const (
	// MsgHandshake — DEPRECATED (per audit F3). Duo agents avoid in new
	// emits; the "." HANDSHAKE-TERMINATOR pattern uses MsgUpdate or
	// untagged compact format. Kept Valid() for hub-history compatibility.
	MsgHandshake MessageType = "handshake"

	// MsgQuestion — DEPRECATED (per audit F3). Duo agents avoid; use
	// MsgCommand for asks-with-action-required or MsgUpdate for asks-as-
	// information. Kept Valid() for hub-history compatibility.
	MsgQuestion MessageType = "question"

	// MsgResponse — answer to a prior MsgQuestion or MsgCommand. Used in
	// BRAIN-cycle replies (Brian↔Rain). [HR] tagging varies by content.
	MsgResponse MessageType = "response"

	// MsgCommand — directive requiring action by recipient. Usually [HR]
	// (recipient must act). Used by user→agent and agent→agent dispatch.
	MsgCommand MessageType = "command"

	// MsgUpdate — informational state-change emit. Catch-all in current
	// usage (~70% of session traffic per audit F1). Default untagged
	// compact-eligible.
	MsgUpdate MessageType = "update"

	// MsgResult — outcome of a prior task/command. Underused vs MsgUpdate
	// (audit F2). Should be used for commit/PR/test-pass announcements;
	// untagged compact for agent-to-agent, [HR]-verbose for GitHub-bound.
	MsgResult MessageType = "result"

	// MsgError — failure / exception state. Always [HR] (decisions usually
	// required). Underused vs MsgUpdate per audit F2.
	MsgError MessageType = "error"

	// MsgFlag — elevated alert. Always [HR]. Hub special-cased
	// (hub.go:153/228) — triggers Discord notification path.
	MsgFlag MessageType = "flag"

	// MsgPeerHalt — peer-coord halt request: drift observed in target
	// agent's recent activity (class-split fire / OUTBOUND-MISS pattern /
	// per-instance-fire-greenflag skip / R12 BRAIN-2nd skip / anchor-
	// checksum mismatch / pm-treated-as-broadcast / force-push-without-
	// elevated-gate).
	//
	// Recipient finishes current tool call → standdown → re-reads
	// discipline-anchors.md → calls VerifyDisciplineAnchor (K-12) →
	// replies with MsgPeerHaltAck carrying recovery summary.
	//
	// Authorized by user msg 6396 (Phase K open) — bilateral
	// halt-each-other-on-drift authority granted to brian + rain.
	//
	// Content payload: JSON via PeerHaltPayload struct (see peerhalt.go).
	//
	// Phase K K-17.
	MsgPeerHalt MessageType = "peer-halt"

	// MsgPeerHaltAck — recipient ack of MsgPeerHalt: standdown completed,
	// discipline-anchors.md re-read, AgentState verified.
	//
	// Content payload: JSON via PeerHaltAckPayload struct (see peerhalt.go).
	//
	// Phase K K-17.
	MsgPeerHaltAck MessageType = "peer-halt-ack"

	// MsgCompactNotice — agent is about to autocompact (Claude Code
	// PreCompact hook signal). Peer agents use this to halt-or-wait on
	// in-flight cross-coord since the compacting agent's in-context
	// memory is about to fragment. Discriminator-broadcast: [HR]-tagged
	// when peer has active fire-in-flight (current_task non-empty);
	// untagged when both peers idle.
	//
	// Phase S S-2.
	MsgCompactNotice MessageType = "compact-notice"

	// MsgResume — symmetric pair to MsgCompactNotice: agent has
	// completed autocompact + reloaded context, signaling peers can
	// resume cross-talk safely. Always-untagged compact-class
	// (informational; resume is not user-attention-warranting).
	//
	// Phase S S-2.
	MsgResume MessageType = "resume"
)

// ActiveMessageTypes lists the 6 duo-recommended types per
// Phase J T1.7 (B7) codification. Use these in new emits; deprecated
// types (MsgHandshake/MsgQuestion) remain Valid() for hub-history
// compatibility but new agent code avoids them.
var ActiveMessageTypes = []MessageType{
	MsgResponse,
	MsgCommand,
	MsgUpdate,
	MsgResult,
	MsgError,
	MsgFlag,
	MsgPeerHalt,      // Phase K K-17 — bilateral halt-each-other-on-drift
	MsgPeerHaltAck,   // Phase K K-17 — peer-halt recipient ack
	MsgCompactNotice, // Phase S S-2 — autocompact PreCompact hook signal
	MsgResume,        // Phase S S-2 — post-compact context-reloaded signal
}

// DeprecatedMessageTypes lists the 2 legacy-preserved types.
var DeprecatedMessageTypes = []MessageType{
	MsgHandshake,
	MsgQuestion,
}

func (m MessageType) Valid() bool {
	switch m {
	case MsgHandshake, MsgQuestion, MsgResponse, MsgCommand, MsgUpdate, MsgResult, MsgError, MsgFlag,
		MsgPeerHalt, MsgPeerHaltAck, MsgCompactNotice, MsgResume:
		return true
	}
	return false
}

// IsActive reports whether this MessageType is in the active set
// (post-Phase-J-T1.7 codification). Returns false for deprecated types.
func (m MessageType) IsActive() bool {
	for _, t := range ActiveMessageTypes {
		if m == t {
			return true
		}
	}
	return false
}

// IsDeprecated reports whether this MessageType is legacy-preserved
// only. Duo agents emit warnings if they see new traffic of these.
func (m MessageType) IsDeprecated() bool {
	for _, t := range DeprecatedMessageTypes {
		if m == t {
			return true
		}
	}
	return false
}

type SessionMode string

const (
	ModeBrainstorm SessionMode = "brainstorm"
	ModeImplement  SessionMode = "implement"
	ModeChat       SessionMode = "chat"
)

// MessageClass classifies a hub message by sender+recipient axis for the
// PM-vs-broadcast authorization-input discipline (R25). Determines
// whether the message can authorize execute-class actions (commit /
// push / merge / gh-pr-create / gh-issue-create / etc.) per
// IsAuthorizationEligible.
//
// Phase K K-18. Closes the "no actions on pms" lost-discipline class
// surfaced by user msg 6391 — peer PMs are coord-only, not authorization,
// regardless of content. Only [HUB:user] broadcasts and [PM:user] direct
// messages authorize execute actions.
//
// Severity tags ([FLAG:*] / [CRITICAL:*] / etc.) are orthogonal to class
// — same MessageClass with different severity. Class itself doesn't
// subdivide on severity.
type MessageClass string

const (
	// MsgClassUserBroadcast — [HUB:user] hub-broadcast from user. Auth-eligible.
	MsgClassUserBroadcast MessageClass = "user-broadcast"

	// MsgClassUserPM — [PM:user] direct message from user. Auth-eligible.
	MsgClassUserPM MessageClass = "user-pm"

	// MsgClassPeerPM — [PM:<peer>] peer-coord PM (brian↔rain). NOT auth.
	// Peer PMs are coordination-class only; receiving agent does not act
	// on PM-implied direction without separate user broadcast/PM.
	MsgClassPeerPM MessageClass = "peer-pm"

	// MsgClassPeerBroadcast — [HUB:<peer>] peer-broadcast (visible to user
	// + other peer). NOT auth — peer broadcasts are status/coord, not
	// directives the receiving agent must act on.
	MsgClassPeerBroadcast MessageClass = "peer-broadcast"

	// MsgClassObservation — [HUB-OBS:<sender>→<recipient>] cross-traffic
	// the observer sees but isn't a direct recipient of. NOT actionable
	// regardless of from-direction.
	MsgClassObservation MessageClass = "observation"

	// MsgClassSystemFlag — emma's auto-emissions: HEARTBEAT-LEDGER /
	// PRE-HALT-SNAP / STALE-CODER / RESUME / [FLAG:emma] halt-fires.
	// System-state pulses, not directives. NOT auth.
	MsgClassSystemFlag MessageClass = "system-flag"
)

// AllMessageClasses lists the 6 message classes per Phase K K-18
// codification. Schema-lock test asserts the closure (catches
// accidental class removal/rename without test update).
var AllMessageClasses = []MessageClass{
	MsgClassUserBroadcast,
	MsgClassUserPM,
	MsgClassPeerPM,
	MsgClassPeerBroadcast,
	MsgClassObservation,
	MsgClassSystemFlag,
}

// IsAuthorizationEligible reports whether a message of this class can
// authorize execute-class actions. Returns true only for user-class
// messages ([HUB:user] broadcast or [PM:user] direct). Peer / observation
// / system-flag classes never authorize execute actions — those agents
// must hold for explicit user authorization on user-class messages.
//
// Phase K K-18.
func (c MessageClass) IsAuthorizationEligible() bool {
	return c == MsgClassUserBroadcast || c == MsgClassUserPM
}

func (s SessionMode) Valid() bool {
	switch s {
	case ModeBrainstorm, ModeImplement, ModeChat:
		return true
	}
	return false
}

type SessionStatus string

const (
	SessionActive SessionStatus = "active"
	SessionPaused SessionStatus = "paused"
	SessionDone   SessionStatus = "done"
)

func (s SessionStatus) Valid() bool {
	switch s {
	case SessionActive, SessionPaused, SessionDone:
		return true
	}
	return false
}

type Agent struct {
	ID         string      `json:"id"`
	Name       string      `json:"name"`
	Type       AgentType   `json:"type"`
	Status     AgentStatus `json:"status"`
	Project    string      `json:"project,omitempty"`
	Meta       string      `json:"meta,omitempty"`
	Registered time.Time   `json:"registered"`
	LastSeen   time.Time   `json:"last_seen"`
	// RebuildGen records the hub's rebuild generation at the time this
	// agent registered. Compared against hub.CurrentRebuildGen at read time
	// to detect pre-rebuild stale registrations leaking into post-rebuild
	// state. Zero on legacy rows that predate the column.
	RebuildGen int64 `json:"rebuild_gen,omitempty"`
	// CurrentTask declares an active multi-step work-thread (Phase-R-
	// followup (f)). Empty string = no current task. Non-empty value
	// signals intentional-idle to emma-stale checker, which suppresses
	// stale-coder PMs for this agent until cleared. Agent-side write
	// via hub_set_current_task MCP tool at work-start; clear via empty
	// string at work-end.
	CurrentTask string `json:"current_task,omitempty"`
	// AgentSessionID is the Z-3 binding to a per-session container
	// (sessions-as-containers). Set at register-time from the
	// BOT_HQ_SESSION_ID env var of the spawning tmux session. Empty
	// for global agents (emma, clive, discord). Distinct from the
	// top-level Message.SessionID field which routes individual
	// messages.
	AgentSessionID string `json:"agent_session_id,omitempty"`
}

type Session struct {
	ID      string        `json:"id"`
	Mode    SessionMode   `json:"mode"`
	Purpose string        `json:"purpose"`
	Agents  []string      `json:"agents"`
	Status  SessionStatus `json:"status"`
	Created time.Time     `json:"created"`
	Updated time.Time     `json:"updated"`
}

type Message struct {
	ID        int64       `json:"id"`
	SessionID string      `json:"session_id,omitempty"`
	FromAgent string      `json:"from_agent"`
	ToAgent   string      `json:"to_agent,omitempty"`
	Type      MessageType `json:"type"`
	Content   string      `json:"content"`
	Created   time.Time   `json:"created"`
}
