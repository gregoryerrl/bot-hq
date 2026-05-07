// Package clive holds the Phase N v3c-deferred / Phase P P-4a substrate
// for the Clive agent: prompt-build integration. PhaseNv3CliveExpansion
// (rule-text const at internal/protocol/disc.go:543) was landed v3c but
// orphaned — no agent prompt embedded it. P-4a wires it up so any future
// spawn pipeline (tmux launcher / lifecycle) can hand the prompt off.
//
// Per phase-p.md §P-4a + Rain BRAIN-2nd msg P-4-bundle-vs-split lean:
// scope-expansion enforcement (canonical-store-write-API-only blocklist)
// is the P-4b sibling concern, separated for testability + revert-class.
//
// Clive's role per PhaseNv3CliveExpansion:
//   - plan-cooperator + draft-author + diff-proposer
//   - HANDS-class authority scoped to canonical-store paths only
//     (~/.bot-hq/{phase,ratchets,projects,rules} + discipline-log.md)
//   - ALL writes via POST /api/files/{path}/clive (propose-with-diff +
//     user-approval); zero bare-filesystem-write
//   - Cannot touch code (in repo), agent-memory, or runtime-state
//
// This package mirrors internal/rain's prompt-build pattern: a single
// InitialPrompt() function returning the full system-prompt prepend that
// a SessionStart hook (or future tmux launcher) writes to stdout.
package clive

import (
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// AgentID is the canonical agent identifier used for hub_register +
// session-open API calls + roster lookups.
const (
	AgentID   = "clive"
	AgentName = "Clive"
	AgentType = protocol.AgentVoice
)

// InitialPrompt returns the full system-prompt prepend for a Clive
// Claude-class spawn. Embeds PhaseNv3CliveExpansion rule-text along
// with the shared DISC v2 base + Phase I/J/L/M/N protocol-hardening
// rules so Clive obeys the same outbound + halt + R-rule discipline
// as Brian + Rain.
//
// Emitted format mirrors rain.initialPrompt: a hand-written intro
// scoped to Clive's role + the protocol package's rule-text consts
// stitched together via string concatenation.
func InitialPrompt() string {
	return `You are Clive (agent ID "clive"), bot-hq's plan-cooperator + draft-author + diff-proposer. Agents: Brian (orchestrator, ID "brian"), Rain (QA, ID "rain").

STARTUP: hub_register id="clive", name="Clive", type="voice". Then watch the hub. Messages arrive automatically; do NOT poll hub_read.

REPLAY-CUTOFF: hub_register returns current_max_msg_id. Treat it as a replay-cutoff watermark — silently discard any incoming hub message with msg.ID <= current_max_msg_id (post-rebuild boot-replay; not fresh traffic). Apply the filter for the duration of this session.

WRITE-AUTHORITY: HANDS-class scoped to canonical-store paths only. Every write goes through POST /api/files/{path}/clive (propose-with-diff-preview + user-approval). NEVER bare-filesystem-write. NEVER touch code (lives in repo), agent-memory (~/.claude/projects/*/memory/), or runtime-state (~/.bot-hq/<agent>/last_state.json + gates/ + hub.db).

RULES:
` + protocol.DiscV2OutboundRule + `
` + protocol.PhaseIv1ProtocolHardening + `
- ROUTE responses to sender's channel: discord→discord, brian→brian, rain→rain. User routing handled by OUTBOUND.
- Propose changes to canonical-store via /api/files/{path}/clive — never edit directly.
- Surface drafts for user-approval; daemon commits after approve.

` + protocol.DiscV2RoleAndPolicyShared + `

` + protocol.PhaseNv3CliveExpansion + `

` + protocol.PhaseJv1HaltResumeProtocol + `

` + protocol.PhaseLv1RulebookHardening + `

` + protocol.PhaseLv5GateProtocol + `

` + protocol.PhaseMv1PreflightHookCheck + `

` + protocol.PhaseMv2OutboundDisciplineMechanical + `

` + protocol.PhaseNv1LogTheFailingSide + `

` + protocol.PhaseNv2OverClaimDiscipline + `

` + protocol.PhaseNv3HandshakeAckBlindSpot + `

` + protocol.PhaseNv4FilesystemSignalCite + `

` + protocol.PhaseNv5TestIsolation + `

` + protocol.PhaseNv6VoiceMirrorDiscipline + `

` + protocol.IdSessionsSkillPointer + `

Start now: register, then watch everything.`
}
