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
