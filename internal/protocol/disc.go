package protocol

// DiscV2OutboundRule is the locked routing-rule text embedded in agent
// system prompts (Brian, Rain). Bug #1 (Bridge follow-up) bundle
// extracted this from the inline prompt strings in brian.go and rain.go
// to a single source so future drift requires touching exactly one
// location. The ratchet tests in disc_test.go pin four canonical
// literals so any wording change that breaks the rule's substance
// fails CI before landing.
//
// History: the original prompt text duplicated this rule across both
// agent files (brian.go, rain.go) — a dual-edit failure surface even
// with literal-substring tests on each prompt independently. The
// extraction collapses the source of truth; the wiring tests in
// brian_test.go and rain_test.go assert that each prompt embeds this
// const, so a refactor that drops the reference is also caught.
//
// DISC v2 audience-driven routing locked at msg 2147; final wording
// converged at msg 2325 / 2327; const extraction proposed at msg 2326,
// accepted at msg 2328.
const DiscV2OutboundRule = `- OUTBOUND: every reply is a hub_send tool call. Freeform tmux text = invisible. If you answered in pane without hub_send, you did not answer. Backfill immediately.
  - Routing is determined by intended audience. If user is one of the intended audiences, broadcast (hub_send with empty ` + "`to`" + `). If only peer(s), ` + "`to: \"<peer>\"`" + `. Peer reads broadcasts too — never double-send.
  - Peer-coordination (concurs, holds, handshakes, alignment acks) defaults to peer-route. Broadcast reserved for state changes the user needs to see. If a message is both peer-coordination and user-actionable, broadcast.`

// PhaseIv1ProtocolHardening is the bundled token-saving + clarity-discipline
// rules added in the Phase I cycle (post-Phase H + BONCOM cycle, 2026-04-28).
// Captures conventions that emerged organically during high-throughput
// BRAIN-cycle work but weren't formally codified — observed as friction
// points consuming context tokens without commensurate signal value.
//
// Embedded in both brian.go and rain.go prompts; wiring tests pin embedding
// + content shape (per-rule grep-assertions catch accidental rule-deletion).
//
// Companion infrastructure: `~/.bot-hq/ratchets/active.md` ledger file holds
// the cross-cycle forward-ratchet items so agents reference by item-# in
// messages instead of re-listing 28+ items every cycle.
//
// Phase I cycle locked at msgs 4664/4666/4668/4675/4677/4680/4682; const
// scope set at 9 rules (R1-R9) per joint BRAIN-cycle convergence. R9
// AUDIENCE-CLASS-DISCRIMINATOR added msg-block 4686-4704: explicit `[HR]`
// content-prefix tag (M-A) marking must-read messages, untagged default
// compact-eligible. Discriminator wording locked as prescriptive ("must user
// read?") not descriptive ("is user reading?") at msg 4703; the distinction
// shrinks the HR set to user-action-required artifacts only (broadcasts user
// passively observes don't require `[HR]`).
const PhaseIv1ProtocolHardening = `PHASE-I PROTOCOL HARDENING (token-saving + clarity discipline):
- HANDSHAKE-TERMINATOR: peer-ack with no new content + no action needed by either side → emit ` + "`hub_send`" + ` with content ` + "`\".\"`" + ` (single period). The pane response should also be minimal (just the dot or a short note). The terminator IS sent via hub_send so it has a hub.db record + doesn't trip OUTBOUND-MISS; the saving is in the body length, not in skipping the tool call. Loop terminates when both sides emit ` + "`.`" + ` once. Avoids the ` + "`[Acked. standing by.]`" + ` → ` + "`[Concur.]`" + ` → ` + "`[Acked.]`" + ` infinite-handshake pathology.
- CROSS-TIMING-DEDUP: when you read a peer's recent message (via hub_read polling OR system-reminder inbound) and find it covers content you've drafted-but-not-yet-sent, post terse ` + "`[crossed in flight — see msg N]`" + ` (or single ` + "`.`" + ` if peer's content fully covers yours) instead of full repost. The ~30s window is observational — there's no detection layer; rely on hub_read polling discipline + system-reminder visibility. Don't double-broadcast equivalent content.
- QUOTE-TRIM: do not quote >2 contiguous lines of a peer message inline. Reference msg_id + 1-line gloss instead (e.g., ` + "`[~4242]`" + ` shorthand or "concur msg 4242 framing on X"). Exception: explicit diff-context where you're showing what changed in your reading of the peer message.
- SNAP-GATING: SNAP block (Branches/Agents/Pending/Next) emitted ONLY on phase-transition events: commit-land, PR-open, halt-ack, session-close, BRAIN-cycle conclusions where state materially changed. NOT on routine progress updates, intermediate verifications, or peer-acks. If would-be SNAP is ~85% identical to the previous SNAP, skip the block; emit only the delta in prose.
- BRAIN-CYCLE-RESPONSE-SHAPE: when user is in audience, BRAIN-second responses follow the compact pattern: (1) one-line concur/pushback header; (2) per-item, 1-2 sentences if concurring or paragraph if pushing back; (3) one-line greenlight footer. DROP: restating the original asks back to the asker (peer has the message), ceremonial all-caps bullets like ` + "`**STRONG CONCUR**`" + `, padding paragraphs that just rephrase the concur. Pushback gets full prose; concur gets terse acknowledgment. Saves ~30-40% on routine BRAIN-cycle exchanges without losing pushback signal. (For peer-only BRAIN-cycles where user is not reading, see AUDIENCE-CLASS-DISCRIMINATOR — compact format eligible.)
- TOOL-RESULT-DISCIPLINE: default to ` + "`Read`" + ` with offset+limit for files >100 lines. Use ` + "`Grep`" + ` for keyword-search before ` + "`Read`" + ` when looking for specific patterns. For large logs/JSON outputs, sample-then-targeted-read. Avoid Read of 200+ line files when 30 lines are relevant. Tool-result tokens count against context budget the same as message tokens.
- SUBAGENT-DISPATCH: for investigations spanning >3 files OR a single file >500 LOC, prefer Task-tool subagent dispatch with explicit scope + report-budget (e.g., "report findings in <500 tokens"). Subagent's read-cost is isolated; main agent context only sees the summary. Especially valuable when investigation outcome may not require the full file detail.
- COMPACT-COMMIT-FORMAT: for agent-to-agent commit/PR announcements (untagged peer messages), default to the pipe-separated ` + "`sender|event:value|key:value|...`" + ` format. Example: ` + "`brian|commit:f047c36b|files:1|+4/-1|tests:622/622|disguise:clean|next:rain-brain-2nd`" + `. Verbose human-readable required for ` + "`[HR]`" + `-tagged commit artifacts: PR descriptions, GitHub commit messages, issue/PR bodies, commits user must review. Discriminator: per AUDIENCE-CLASS-DISCRIMINATOR — must user read this for review/decision? Yes → ` + "`[HR]`" + ` + verbose prose. No → untagged + compact pipe format. Receiver-can-parse-from-fields-alone test still applies for compact: agent receivers parse via documented schema; user readers need prose context.
- AUDIENCE-CLASS-DISCRIMINATOR: messages default to compact format. Sender marks must-read messages with ` + "`[HR]`" + ` prefix in content body. Discriminator: "must the user read this for correctness or decision?" Yes or unsure → ` + "`[HR]`" + `. Confident no (user can observe but doesn't need to act on this specific message) → no tag. Note: prescriptive ("must read") not descriptive ("is reading") — passive observation ≠ required reading.
  - ` + "`[HR]`" + ` MUST-READ (verbose human-readable required): hub_flag elevations awaiting user-decision; direct user PMs requesting decision/concur; EOD content; PR/issue/comment bodies destined for GitHub; commit messages user reviews; final proposals where user-direction is the next step; any artifact a non-agent will read for action.
  - UNTAGGED (compact eligible): agent-to-agent PMs; peer-acks; agent-side BRAIN-cycle exchanges; trio coordination during user-AFK windows; broadcasts user observes passively but isn't required to act on per-message; HUB-OBS cross-traffic between agents user follows but doesn't direct. Compact format options: positional pipe (` + "`sender|target|event|payload|next`" + `), key-value (` + "`cgr c1e3d729 +71-26 tst:549 next:rain-2nd`" + `), or ad-hoc shorthand. For ad-hoc shorthand: first use per session MUST include a 1-line schema decoder inline (e.g., ` + "`[schema: tst=tests-pass-count, dgs=disguise-clean]`" + `) so future-session audit-replay can parse without archaeology. Schema documentation for the standard formats lives in this prompt const.
  - Sanity anchor: hub_flag elevations, EOD/PR/issue/commit external artifacts, direct user PMs awaiting decision are presumptively ` + "`[HR]`" + `. Broadcasts user follows passively are NOT presumptively ` + "`[HR]`" + ` — passive observation ≠ required reading. When unsure, tag ` + "`[HR]`" + `; false-tag costs tokens, false-untagged costs comprehensibility, comprehensibility loss is worse.`

// H13ForcePushProtocol is the locked text describing how Brian relays
// force-push requests through the user-token gate. Coders configured under
// project rules with force_push_blocked must request via the protocol; Brian
// is the authority that verifies the user's verbatim token reply before
// relaying greenlight back to the coder.
//
// Phase H slice 1 / H-13 — locked in coordination with rules.ForcePushBlocked
// (per-project) and the FORCE-PUSH POLICY section in coder preambles
// (internal/mcp/tools.go buildCoderPreamble).
//
// Brian-only: Rain does not dispatch coders, does not relay force-push
// tokens. The constant ships in brian.go's prompt only.
const H13ForcePushProtocol = `H-13 FORCE-PUSH TOKEN PROTOCOL (when coder PMs you with ` + "`request_force_push: <branch>@<sha>`" + `):
- hub_flag user with the request + the EXACT verbatim token they must reply with: ` + "`force-push-greenlight: <branch>@<sha>`" + `.
- WAIT for user reply. Do NOT auto-construct or coach the token text — user must type it themselves.
- Verify: user's reply must contain the EXACT string ` + "`force-push-greenlight: <branch>@<sha>`" + ` with the SAME branch name and SAME SHA from the original request. Substring search of the user's reply is fine; partial matches are NOT acceptable.
- On exact match → hub_send to coder: "force-push approved for <branch>@<sha>". Coder may then push.
- On miss / mismatch / wrong-sha / wrong-branch → hub_send to coder: "force-push DENIED — token did not match". Coder must NOT push.
- This gate exists to prevent destructive force-pushes in client projects (boss-safety class). Never bypass.`
