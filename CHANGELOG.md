# Changelog

Releases, newest first — [Keep a Changelog](https://keepachangelog.com/) shape.
Work between releases accumulates under **[Unreleased]**; a release moves that
block under its version heading. Development history before 1.0.0 lives in git
and in `docs/rebuild-archive/`.

## [Unreleased]

A second connector is on its way in: OpenAI's Codex CLI beside claude-code,
chosen per saved model, so one session can run a Claude model and a GPT
model side by side. Migration 0102 applies on first launch.

Managed sub-agent development is in verification. Roles now offers an opt-in
**Sub-agent development** capability: up to two read-only helpers, selected by
saved model/account, usable by either parent runtime. Claude-connector models
include configured gateways; Codex helpers are supported on Unix. Starts return
immediately; bounded status waits and stable request IDs prevent duplicate
launches after a lost reply. Helpers stop with their parent's turn and leave
recent evidence in one bounded document; retained findings and compact traces
remain readable by reviewers. Final findings survive long commentary and file
results. The parent still integrates and verifies the
work. Codex helpers require explicitly disabled MCP servers with empty inventories;
only built-in empty listing calls are allowed. Newly started Claude participants
no longer expose native Agent/Workflow/Task delegation, with or without the checkbox;
the checkbox grants managed helpers only. Existing processes adopt this at restart.

### Fixed

- A forced approval (`action_gate` with `require_approval: true`) is read by
  the reviewer before it reaches your tray, whatever the command's shape —
  scripts included, which bot-hq does not parse. Three production reads
  wrapped in Python scripts had reached the tray with no review turn (#127).
  The card now says whether the review happened and by whom, or why it was
  skipped (no other reviewer, a reviewer-down override, a roster change). An
  urgent forced approval notifies you at once and again when it parks; review
  never extends a run window. An explicit local approval now waits one review
  turn too. An identical re-issue returns the same gate; one already parked
  without a read is named in the reply instead of queued twice. A command a
  reviewer vetoed is refused unchanged until the finding is answered (listed
  reads too), and a review notice too large to reach the reviewer whole is
  refused before anything queues.
- Close-out turns no longer inherit the preceding turn's displayed age or
  unanswered tool indicator. Restarting host prompts remain system-attributed
  in chat; passive notices do not reset the display. Before new agent output,
  the prompt's time is the fallback, not an exact turn-start measurement.
- Plain `gh api` GETs can pipe into `rg --no-config` with supported search and
  output flags in agent shells without an extra approval. Host-run `read_gate`
  refuses rg before parking and points to grep, since agent-bundled tools may
  not exist on the host. The flag must come first;
  config-driven execution, program-launching options and unknown flags are not
  included in this exemption.
  Read pipelines also refuse unquoted glob/brace expansion, which can insert
  extra options; quote regexes and GitHub placeholder/query paths.
  The exemption accepts bare gh/filter names, exact Unix /bin and /usr/bin
  tool paths, and a plain `command <filter>` prefix. Arbitrary executables
  such as ./rg or /tmp/head cannot impersonate a read-only tool.
- Read-only filters on a plain `gh api` GET or a listed read now transform
  only their pipe: a file operand (`grep . /path/prod.env`), a file-loading
  option (`grep -f`, `jq --rawfile`, `wc --files0-from`, `rg -f`) or a form
  that leaves stdin (`grep -r`, `tail -F`) keeps normal gate handling instead
  of riding the exemption, and the refusal names the file or option. Unknown
  filter options are refused rather than assumed harmless; `-` still means the
  pipe in a file position. A long option's value must be attached for the
  coreutils filters (`--context=3`), since BSD and GNU disagree on optional
  values, and a jq program that `import`s or `include`s a module is refused
  (that loads files with no option at all). Every recorded `gh api` command
  classified as before.
- Messages sent to agents quote bracket-led body lines at visual line starts, including untimed and
  phase-like tags, fullwidth brackets, Unicode line separators, and invisible
  prefix formatting. Stored content stays unchanged, and the escaped view
  keeps its delivery cap. This guards transcript framing, not arbitrary
  instructions embedded in untrusted text.
- A body's first line stays unquoted when it shares the real speaker header's
  line; phase, banner or prefix framing puts it at a new line and enables quoting.
  This preserves host notices and review-notice deduplication without exempting
  a sender or trusting later forged tags. Full host instructions remain visible
  in chat; idle and close-out instructions use a short trusted heading.
  Close-out prompts are attributed to the system, never to the human.
- bot-hq's own MCP connection now allows 660 seconds for a tool reply, so a
  permitted direct command can complete beyond the clients' previous 60-second
  cutoff. Command bounds stay at 120 seconds by default and 600 maximum; other
  MCP servers keep their configured timeouts. A stalled bot-hq tool may now hold
  its caller for up to 11 minutes.
- Direct host commands stop when their issuing runtime is interrupted, retired
  or closed, even if its native MCP client leaves the HTTP request open. Atomic
  commands and outward publications retain an eight-second finish grace across
  interrupt sources; time already spent deferring Pause counts toward it.
  Routine direct outcomes get a one-line receipt without rebroadcasting output.
  A stopped caller gets the actual result if it finishes during grace, or an
  explicit unknown outcome with cleanup requested if interrupted. Grace never
  starts new work after a caller disconnects. Approved background gates keep
  their existing lifecycle.
- On Unix, host-run commands own a detached session. A command bound or cancelled
  request now stops the whole process group, allowing two seconds each after
  interrupt and termination signals before force-killing. This gives clients a
  chance to cancel remote work; it does not guarantee a remote query stops.
  Git commits, pushes and migrations therefore stop at their bound instead of
  continuing as orphaned children. Signal-based app stop/relaunch also kills
  running host commands, including approved commands. Normal window close/Cmd-Q
  and Windows descendant cleanup are not covered by this change.

- Codex models that require code mode can discover and call application tools,
  including managed helpers. Startup no longer excludes MCP tools from that
  route, and runtime instructions explain how to find and call them.

- A failed close no longer leaves a stale close-out claim that interferes with
  later closes. Reopen reports that close-out is still finishing while its
  teardown owns the session, preventing an old close from killing a reopened
  session. The claim releases after cleanup on either success or failure.


- **A respawned reviewer drops the old process's dead status.** Its new event
  pump publishes current liveness at startup, so a healthy replacement does
  not become commit-blocking again when its last tool call is a minute old.
  Retired pumps cannot reset the replacement's health; an actual exit still
  marks it dead and blocks commits.

- **Codex account folders work through filesystem aliases.** Hook identity
  checks accept the same file through paths such as `/tmp` and `/private/tmp`
  or a directory symlink. The configured account path stays unchanged, and
  cached trust preserves the exact key and hash Codex reported. Different
  files, hook positions, commands, events and matchers remain rejected.
- **A fresh daily report no longer retires yesterday's code vocabulary.**
  The close-out sweep now requires an explicit retirement declaration or a
  library file actually removed or renamed. Dropping names, filenames or
  timestamps from an EOD produces no warning. Current references to an
  explicitly retired name still report; historical records and retirement
  explanations are preserved. The separate repository-reference scan remains
  available through `cl_stale_refs`.
- **Codex participants know project startup hooks need manual recovery.**
  Their startup guidance states that project `.claude` hooks are not loaded
  automatically and tells them to check required handoff markers and follow
  the project's recovery instructions without claiming a hook ran.
- **Gate results stay visible when a handoff is newer than their completion.**
  The issuer receives a once-per-result notice on a subsequent MCP reply,
  and both connectors restore bounded result excerpts after compaction
  until the turn ring delivers them. A notice does not mark the result read.
  Delivery proof survives telemetry cleanup; rejected/refused gates and
  later scheduled results use the same receipt path. Full output remains
  available through `gate_status`.
- **A close approval belongs to its card.** An old Approve can no longer
  authorize a later close request after new work or a rejection. Concurrent
  requests share one current card. “Close and archive” is now stored on that
  card and survives review, retry and restart; a retry cannot change what the
  user is being asked to approve. An unchanged old shipped close paragraph
  is upgraded inside customized HANDS instructions; surrounding custom text,
  edited paragraphs and capabilities are preserved.
- **Close responses describe the card that actually exists.** The tool now
  reports its status, card ID and next action as structured data alongside
  its explanation. Repeated calls return the same queued or pending card,
  including after a restart. Advisory details no longer tell the agent to
  retry while a card is open; general guidance uses the single close card.
  User action documents are identified as remaining on the dashboard after
  closure, rather than included in the scratch-document archive warning.
- **Automatic and scheduled command results display as host notices.** They
  no longer appear in the chat as if the user had written their output.
- **Scheduled command results wake the agent that requested them.** The
  result is recorded before queuing a turn, which waits behind live work,
  Pause, declared halts and other approvals. A removed requester passes
  delivery to an active participant; closed sessions receive no wake.
  Duplicate run claims produce no extra result or wake.
- **An internal retry clears the old host halt banner when work arrives.**
  A resumed turn no longer appears halted under an earlier automatic recap.
  Empty or interrupted deliveries keep the recap; agent-declared halts,
  Pause and pending approvals keep their hold.
- **Queued reviews resume when the blocking gate resolves.** A reviewer
  waiting behind a push approval no longer needs an extra user message to
  get its turn. Gate answers are recorded before scheduling resumes;
  each answer wakes the ring once, without a phantom follow-up turn.
  Closing clears the gate without starting more project work. A session
  with no queued work gets a recap, and a running command stays identified
  as running.
- **Tray answers wait for the current turn to finish.** A stale idle indicator
  could let an answer replace a working turn or falsely halt the session.
  Scheduling now checks the actual turn owner. A new answer also starts fresh
  pass accounting, so an earlier all-pass halt cannot swallow it.
  Answers wait for the turn to end even when it takes several minutes;
  Pause remains the way to interrupt it immediately.
- **An interrupted turn cannot finish a newer turn.** A completion arriving
  after a new deal keeps the interrupted turn's identity, including when the
  old turn had not produced output. Its busy flag cannot clear the new turn's,
  and an interrupted or failed review cannot release a queued publish.
  After a transient review failure, the publish waits for the retried turn
  to complete successfully.
- **Resuming after Pause hands out one turn.** An interrupted completion
  held by Pause used to hand out the new message before the resume itself
  handed it out again. That could show an all-pass halt while an agent was
  working, or start two agents at once. Held outcomes are now recorded
  before the resume makes its one deal; pending gates and mentions survive.
- **The first message after a relaunch lands.** Opening a session that
  was dormant (after a relaunch, or one nobody had looked at) starts its
  agents in the background, and a message sent in those seconds failed
  with "no live session" and had to be sent again. Send now waits for the
  agents to be up — the same wait a staged tray answer gets, before any
  answer is recorded.
- **A gate id quoted in chat shows how its run ended.** The chip beside a
  gate id read "approved" even when the approved command then failed, timed
  out or was refused at run time; it now reads "ran", "exit N", "timed out"
  or "refused" (with the reason in its tooltip), and "approved" only while
  the run has not ended.
- **A respawned participant's first turn counts.** When a session's roster
  came back (opening the session, a relaunch) a Codex participant announced
  itself before any turn was dealt, and that announcement was taken for the
  start of a turn — so its first real turn ended as a completion the ring
  threw away, and the session sat stalled until the idle watchdog nudged
  it. Nothing said before the ring deals a turn opens one now.
- **Session Settings survive a relaunch.** The per-session policy snapshot
  (the gear tab's Tool-Gate keywords and push toggles) was wiped for every
  session at startup, so each relaunch reset what you had allowed in an
  open session. Only closed sessions' snapshots are cleared now.

### Added

- **Driver control groundwork (in progress).** Roles has an explicit application
  control grant, never enabled by default. Its first API reads sessions, history,
  documents, decisions and an attributed action audit across sessions. Current
  role revocation and human Pause are checked on each call. Gate/question answers
  now share the normal claim/execution/wake path with the controller's identity;
  a queued answer loses to Pause, revocation or replacement, and requires the
  exact choice hash read by the controller. Delegated answers never become
  recorded human words. Delegated messages now queue without interrupting
  workers or replacing the human's staged text; stable request IDs prevent
  duplicate posts, and receipt lookup reports actual delivery. Pause, gates
  and declared halts remain holds. Fresh model contexts also carry bounded
  recent channel excerpts with source attribution, so delivered Driver directions
  do not disappear merely because they are not human-authored. The same history
  cutoff keeps startup-time messages out of this snapshot. Controllers can now
  start dormant open sessions and explicitly resume started sessions without
  interrupting a live worker. Closed sessions still require reopening; start
  does not lift Pause. Recipient guidance recognizes authenticated delegated
  directions while keeping human attribution separate. Controllers can also
  create sessions with the dialog's participant and worktree choices. The result
  distinguishes creation from startup failure; an existing ID cannot be
  overwritten, and queued creation loses to Pause or permission revocation.
  Creation also rejects unknown explicit model IDs instead of silently falling
  back, and rejects misspelled participant fields.
  Controllers can now pause, close/archive and reopen sessions through the normal
  application controls. Self-pause and self-close record their outcomes before
  interruption; reopening preserves history and phase and leaves an explicit
  halt for the next direction. Driver administration now covers models, roles,
  app settings, policies and Tool-Gate keywords, using the desktop validation
  rules and preserving frozen session settings. Saved model tokens are kept
  unless explicitly replaced or cleared; endpoint, provider, runtime and CLI
  settings changes require explicit credential intent when a token is stored.
  Partial edits preserve unmentioned role instructions,
  account/model fields and policy rules, and role saves can reject stale hashes.
  Audit records identify the changed role, model, setting or policy scope.
  Internal telemetry/preset state cannot be changed through the generic settings
  endpoint. Roster changes use the same safe
  boundary as the Session Settings controls; revocation while queued cancels
  them before a process is replaced and posts a cancellation follow-up.
  Configuration changes refresh the desktop.
  Event watches, relay and shared-webview controls are still being connected.


- **Agents can retrieve older records from their own session.** The
  `session_history` tool pages raw channel history or the recorded human
  instructions and tray choices separately. Long records can be read in
  bounded chunks. Retrieval requires channel-read permission and changes
  neither the turn cursor nor delivery receipts; context-transfer prompts
  point to it for omitted text.

- **Add, remove and change agents in Session Settings.** Choose a saved role
  and model, or switch an existing agent to another model, provider or account.
  Changes wait for a completed turn; pending requests can be cancelled, and
  progress or failure remains visible after reopening Settings. The last active
  agent cannot be removed. Other agents keep their conversations; history,
  findings and pending gates stay available after removal.
  Model changes start fresh context with bounded, attributed excerpts and a
  usage warning. Startup failure restores the prior configuration and runtime;
  an unrecoverable failure leaves a recovery recap. Current roster facts are
  refreshed in compaction handoffs without changing their author dates, and
  only the affected agent's live context meter is reset.
- **Auto-allowed publications still receive review.** Both connectors and
  `action_gate` queue unread publication content for the reviewer, then the
  background worker runs it without a user approval card. The queue survives
  restarts, checks the body, scripts and current policy, and reports automatic
  execution without inventing a user answer. Forced approvals, listed data
  reads, urgent requests and run windows still require approval. Retries share
  one gate; a stronger request revokes automatic execution. A session without
  a reviewer can run directly; a configured reviewer that is down cannot be
  silently skipped. Review completion never waits for the command to finish.
- **A saved model names the CLI it runs through.** The Model dialog has a
  **Runtime** field — Claude Code (the claude CLI, as before) or Codex
  (OpenAI's Codex CLI). For a Codex row the account field is its **Codex
  home** (the `CODEX_HOME` whose ChatGPT sign-in it bills; give bot-hq a
  home of its own, never `~/.codex`), the dialog shows the one-time
  `codex login` command for it, and the Claude CLI settings field is
  hidden. The list marks Codex rows. The New Session dialog's account
  quick-select stays a Claude-account swap and leaves Codex rows where you
  put them.
- **A participant on a Codex row runs through the Codex CLI.** One
  `codex app-server` process per participant, driven over its JSON-RPC
  protocol, so a session can pair a Claude model with a GPT model (Fable and
  Astra, say) and both take turns in the same ring, read the same documents
  and call the same bot-hq tools. The account it bills is the ChatGPT
  sign-in of its Codex home (recorded at its first spawn, like a Claude
  participant's config dir); its context meter reads the window Codex
  reports; a usage limit, a lost sign-in or a full context is said in the
  channel the way a Claude participant's is. **Test** on a Codex row checks
  the home's sign-in and that the model is one the account offers.
- **A Codex row's context window is the one Codex runs the model at.**
  Codex's own defaults are small (272,000 tokens; 500,000 on GPT-6.1 Sol)
  and well under the API's published windows, and its catalog caps any
  override at 872,000 on the ChatGPT models. bot-hq hands the row's value
  over as `model_context_window` together with a model catalog — Codex's
  own, with the row's window on the row's model — so the published window
  (1,050,000 on the GPT-6 and GPT-5.6 models) is what the model runs at; a
  context of 853,000 tokens was served to GPT-6 Astra this way. Codex keeps
  5% of it back and the meter sees the rest; bot-hq places Codex's
  compaction point below that so the handoff rows come first. A value Codex
  does not take gets the session one notice naming the value it runs.
- **The Tool Gate and the reviewer's deny list hold on Codex.** Codex has
  no permission layer of its own that bot-hq can lean on, so bot-hq writes
  one hook into the Codex home and that hook is the only gate: it asks the
  running app which role is calling, applies the Tool Gate's keywords (and
  parks the command for you) for an executor, and the read-only deny list —
  no commits, pushes, branch or tree mutations, no `gh` writes, no file
  edits — for a reviewer, through every shell form (`cd x && git push`,
  `sh -c '…'`, `xargs`, git's global options). It refuses, loudly, whenever
  it cannot decide (the app not running, an unknown caller) and before
  every turn bot-hq checks Codex still lists and trusts it. Terminal
  sessions (which could carry commands past the hook), Codex's own
  sub-agents, app connectors, plugins, goals, image generation, worktrees,
  browsers and memories are switched off for participants. (The sub-agent
  tools need their own switch: on Codex 0.163.0 the model catalog turns
  them on regardless of the feature flags, and the first Fable + Astra
  session still saw them — bot-hq now sets `agents.enabled=false` too.)
- **Interrupting a Codex participant does not wait for its command.** Codex
  answers an interrupt at once but lets the running command finish, so a
  message typed during a long build would have waited for the build. bot-hq
  now gives the turn a moment (longer mid-commit or push), then stops the
  participant's own Codex process and resumes its thread on the next
  message. The interrupt itself names the turn it stops: without that
  Codex refused every interrupt and only the fallback ever stopped a
  participant (found in the first live sessions, 2026-10-21); a gate hook
  that fails to run now gets the same deadline, and an idle participant is
  not asked to interrupt anything. And because Codex ends the interrupted
  turn but lets its command run on (a Pause left a `sleep 180` running),
  bot-hq now stops the command itself when the turn ends — at once, or
  after the same grace mid-commit or push — before the next turn starts;
  on Windows, where bot-hq cannot walk the process tree, the command still
  runs on. After Codex compacts a participant's
  context, bot-hq puts its handoff document and your words since back, as
  it does for a Claude participant; a used-up ChatGPT usage window is said
  where the session starts. **Test** on a Codex row leaves nothing running:
  the probe's short-lived app-server (the native binary under npm's `codex`
  wrapper) is reaped with its whole process group, on every exit path.
- **An agent can relaunch bot-hq and come back to its session.**
  `scripts/relaunch.sh --from <session> --delay 20` stops the app, rebuilds
  it and launches it again from outside the agent's own process group (the
  app's exit kills that group), refusing while any other session is
  mid-turn. The agent declares a temporary halt first; at boot bot-hq now
  respawns every open session that holds a wake instant, so the halt
  re-arms and its expiry deals the agent its next turn. The same boot step
  fixes a quieter gap: a halt that expired while the app was down (an agent
  waiting on CI, say) used to wake nobody until you opened the session.

## [1.0.10] — 2026-10-19

One look across the app: the chat reads as a conversation, the session header
is two rows, the document tabs wrap, and buttons, fields, chips, dialogs, type
and icons each have one style. A FOR YOU document can be marked done where you
read it. No database migrations: a 1.0.9 database opens unchanged. 1.0.9
stayed a draft, so from 1.0.8 this release carries 1.0.9's changes too, and
migrations 0087–0101 apply on first launch.

### Added

- **A document waiting on you can be marked done where you read it.** A
  FOR YOU document (one an agent wrote for you to act on) could only be put
  away from the dashboard's "Your action items" list. Its own header now has
  **Mark done**; once done it says when, and **Undo** puts it back on the
  list. The dashboard's list follows either way.

### Changed

- **The chat reads as a conversation.** Each run of an agent's tool calls
  is one quiet line — "3 tools · Bash ×2, Read · 14s · 1 failed", or the
  call itself when there is one — instead of a pill of raw JSON per call
  and per result. Open it for one line per call with the first line of its
  result, a mark for passed, failed or running, the files it wrote, a
  thumbnail for a screenshot (never its base64), and the raw call behind
  "raw". A failure is never hidden by the fold. Your own messages sit on
  the right in your blue; the agents' stay on the left under their colour.
  bot-hq's notices are upright and show their warning once.
- **The session header is two rows that never break a participant.** The
  first holds Dashboard, the title, the phase and the actions; the second,
  the session id, the worktree, the account and one chip per participant
  (its colour, ROLE · Model, health, context and backlog), wrapping between
  chips. The account you bill shows once when everyone bills the same one;
  when they differ, each chip carries a short tag (`default`, `acct-2`).
- **The document tabs wrap** onto a second line instead of scrolling
  sideways, so nothing in the app scrolls sideways any more.
- **One look for the parts every screen shares.** Buttons, fields, chips,
  tabs and dialogs each have one style now:
  - buttons are outlined with mono labels, and never submit a form by
    accident;
  - every input, select and text box is one boxed field;
  - status chips share one shape and one set of tints;
  - every modal has the same frame — a close button, a body that scrolls on
    its own and actions that stay in view — and Escape closes only the
    topmost one;
  - every Settings page is one width, with its scrollbar at the window's
    edge.
- **One type scale and one icon set.** The app uses the design's five type
  sizes and nothing else, message prose is set in Inter and data in
  JetBrains Mono, help text is a quiet slate instead of beige, and the text
  glyphs (✕ ▸ ⚠ ✓ ⏸ …) and emoji that stood in for icons are drawn from one
  icon set. The participant colours are tuned to the navy theme, keeping
  their names.

### Fixed

- **A chat opened on a long session starts at its newest message** instead
  of somewhere above it, and opening a run of tool calls keeps its line
  where you clicked.
- **A text selection dragged out of a Context Library dialog no longer
  closes it** when released over the dimmed background.
- **A command's glob or shell syntax is no longer offered as a file to
  view** ("View *.test.tsx").

### Deferred

- **Tailwind CSS 4** (Dependabot #8), as in 1.0.9.
- **The chat composer** keeps its own field metrics; every other input is the
  boxed field.
- **The new look on Windows** is checked by CI's Windows suite, not yet by eye
  on a Windows machine.
- The rest of 1.0.9's Deferred list stands: the review-notes notice, the
  Windows gaps and glib 0.18.

## [1.0.9] — 2026-10-18

A second Claude account beside the first, gates that run inside a time window
or take their command from a file, a handoff that survives a context compaction,
production reads behind your approval, the feedback queue closed from inside
a session, a UI pass, and Windows without a console window. Migrations
0087–0101 apply on first launch (1.0.8 shipped through 0086). Windows users:
`docs/WINDOWS-TESTING.md` lists what to check on your machine.

### Added

- **A second Claude subscription can run beside the first.** A saved model
  can name a Claude config dir (Settings → Models → "Claude config dir"), the
  folder a second account is signed in to; leave it blank for the account in
  `~/.claude`. A participant on that model is started in that folder and bills
  that account, so one session can run on account 2 while another runs on
  account 1 at the same time — pick the "· acct 2" rows in the New Session
  dialog, or set them as a role's default for the day. The dialog shows which
  account each participant will bill, and an "Account" pick above the rows
  moves every participant to the same model on that account where one is
  saved (and names the rows it could not move). A participant keeps the folder
  it started in for its whole life, so resuming never crosses accounts; the
  session header shows the account each participant is on. The Model dialog
  shows the one-time terminal command that signs the account in to its
  folder (`claude auth login --claudeai`) — bot-hq never runs it — and "Test
  connection" on such a row checks that folder's login. Nothing moves work to
  another account on its own.
- **A usage limit is remembered per account and model.** When claude-code
  reports a limit, the account and model that hit it are marked with the
  reset time it gave
  (`resets 8pm`, `resets Oct 5 at 2pm`, read in your machine's time zone). The
  Models list and the New Session dialog say "limited until …" on the rows
  that bill that account on that model (an exhausted Fable says nothing
  about the same account's Opus), a session that starts on it is told once,
  and the mark clears by itself when a later turn on that account and model
  completes — or with a click. It is only ever a warning: nothing is refused and nothing is
  re-routed. The weekly-limit wording is now recognised too (it used to slip
  past the detector).
- **A participant that is not billing its subscription is called out.** If
  claude-code reports that an API key reached a subscription participant (an
  `apiKeyHelper` or an `env` entry in a shared settings file, say), the chat
  says so once at start. A second folder signed in through the Console
  (API-usage billing) rather than the claude.ai login is refused for a new
  participant; in `~/.claude`, or for a resumed participant, it is called out
  instead. Known limit: the check accepts only the
  values claude-code 2.1.284 was measured to report for a subscription
  (`authMethod` `claude.ai`, `apiKeySource` `none`); if a later CLI renames
  them, a new participant's start in a second folder is refused and every
  other start gets the notice — each names the value it saw, so the cause is
  visible rather than silent.
- **A participant keeps its bearings across a context compaction.** Each
  participant has one handoff document, `handoff-<its name>` (for example
  `handoff-hands`), which it writes like any session document and you can read
  and edit in its own tab. When claude-code compacts that participant's
  context, bot-hq puts the document back into it, so the hand-over no longer
  depends on you asking for it or on a peer noticing.
- **The compaction summary carries what happened since the handoff was
  written.** When claude-code compacts a participant, bot-hq now tells the
  summary to end with everything since the participant's handoff document was
  last written: what you said, verbatim and with its time, the decisions, what
  was done and how it was checked, what is in flight, and what in the handoff
  is no longer true. So the hand-over stays current even when the handoff was
  written an hour before the compaction. The handoff put back after the
  compaction now says how long before it the document was written ("last
  written 05:40 — 38 minutes before this compaction") and which source wins
  for what: the summary for anything after that time, the document's own
  words for anything before it.
- **A `gh api` read runs without approval, and a command that only writes a
  file says so.** A `gh api` call with no field, body or method that makes it
  a write — gh's own rule for a GET — is a read like `gh pr view`: it is no
  longer treated as a publish the reviewer must read first, and by your
  choice it runs without approval whatever the Tool Gate keyword list says,
  when it is one command piped at most into head, tail, jq, grep, wc or cut.
  Every write shape still gates: a `-f`/`-F`/`--field`/`--raw-field`/`--input`
  field, `graphql`, `-X`/`--method` with anything but GET, a method-override
  header, a pipe into a shell, or another command on the same line. And when
  a parked command only writes text into a file — a `printf` assembling a
  script, a heredoc — the approval card now says so, so an Approve that runs
  nothing in that text is not mistaken for one that does.
- **A temporary halt can wait up to 48 hours.** `wake_after_secs` on
  `mark_awaiting_user` and `halt` was capped at an hour, so an overnight CI run
  or tomorrow's cron meant re-declaring every hour — and a participant that
  did not left the session halted on you instead. The cap is now 172,800 s
  (48 h); the session tile names the day when the wake is not today ("wakes
  tomorrow 14:05", "wakes Oct 14 14:05"), and the banner's countdown already
  counts hours.
- **A long command can be handed over as a file, with a bound for its run.**
  `action_gate` takes `command_path` — an absolute path whose whole text is the
  command (up to 256 KiB, read once) — so a multi-line script no longer has to
  be squeezed into inline text. The file's text is treated exactly like a typed
  command: the same shell lint, the same approval rules (a file holding a
  `gh issue comment` still waits for the reviewer's read), the same shell runs
  it, and editing the file after parking changes nothing. The reply names the
  file, its size and its sha256, and `gate_status` repeats the hash on every
  gate so what was parked can be checked against the file. `timeout_secs`
  (1–600) bounds an approved command's run; the default stays 120 s, and the
  running mark and `gate_status` say the bound. A long command is echoed to the
  agents as its first lines plus its size and hash — in the reply, in
  `gate_status` and in the answer row — while your approval card keeps the
  full text, so a 256 KiB script no longer costs every participant its whole
  length three times over.
- **What you said comes back word for word after a compaction.** Beside the
  handoff document, bot-hq now puts back everything you typed, staged or
  answered in the tray since that document was last written (the whole
  session's, for a participant with no document), exactly as you wrote it —
  a summary paraphrases, and an instruction paraphrased is an instruction
  changed. The participant is told these words outrank the summary's account
  of them. Only your own words count: bot-hq's resume notice and close-out
  prompt, a plugin's message and a command's output are not put back as yours;
  a tray answer comes back as your pick with the question it answered. The
  newest messages are kept when they do not all fit, and the compaction row
  says how many came back.
- **A compaction is announced in the chat.** A row says whose context was
  compacted, how many tokens it went from and to, and whether its handoff
  document was put back, so the other participants know to check that it
  still holds the session's standing instructions.
- **Agents are told the time.** Every message an agent receives now shows
  when it was posted, in UTC, so an agent reads the time from its newest
  message instead of estimating it. Estimated times had been ending up in
  handoffs and reports.
- **An agent can read what an approved command produced.** Asking for a
  gate's status now returns its exit code, how long it ran and the end of its
  output, and while it is still running says for how long. The reviewer can
  ask too, using the short id shown in the chat. Before, the output reached
  the agent that issued the command only at its next turn, and the reviewer
  had no way to fetch it.
- **An agent can take back a command it no longer wants** while it is still
  waiting for the reviewer's read, so it does not go on to prompt you.
- **A project can list its production and staging reads.** In its policy, a
  project may list the commands that read production or staging data (for
  example `gcloud logging read`, `bq`, `psql -h <host>`). A command that runs
  one of them — in an agent's Bash, an approved command or the terminal — now
  parks for your approval, after the reviewer reads it, and the card says which
  entry matched. A mention of the command in other text does not count. An
  agent can add "approve only after X" to the card when the read depends on an
  earlier step.
- **A read-only SQL query on production, with the SQL on the card.** A
  project can configure its production Postgres database in its policy
  (host, database, user, and where the password lives). An agent's
  `prod_read` then parks a query for your approval with the SQL itself on the
  card, not wrapped in shell. Only reading statements are accepted, the whole
  query runs as one read-only transaction with a time limit, and the password
  is never in the command: it is read from your file when you approve. A
  setting still holding a `<placeholder>` counts as unset: the call is refused
  at once, naming the settings to fill in. SQL
  that could be read two ways (a nested comment, an escape string, a
  backslash) is refused, and bot-hq's own first statements fix the
  transaction as read-only before yours run. Calls to functions that act
  outside the transaction (ending other sessions, `dblink`, changing settings)
  are refused. A read-only database user is still the real boundary: a
  function your database defines can reach outside the transaction.
- **The reviewer can ask for its own production read.** It had no way to park
  a read for your approval, so it either ran production reads directly or
  routed them through the agent it was reviewing. A new `read_gate` tool takes
  one listed read (piped at most into a read-only filter like `head` or `jq`,
  and with no write verb such as delete or deploy) and parks it for you, marked
  as the reviewer's read. It does not take a database client like `psql`, whose
  own commands it cannot check (`prod_read` is the path for SQL), and takes a
  `bq query` only as one plain SELECT or WITH. A listed read typed straight
  into the reviewer's Bash is stopped and pointed there.
- **A publish you approved is read back.** After an approved `gh` issue, pull
  request or comment publish runs, bot-hq reads what landed on GitHub and
  says in the result whether it equals what was reviewed, or where it first
  differs. A pull request also says which issues GitHub will close with it.
  Reviewers had been checking every publish by hand. For an edit, the
  reviewer now sees the diff against the text currently on GitHub, and the
  approved edit is not run if that text changed after the review.
- **A zsh command that would not do what it says is stopped before it runs.**
  Under zsh, `"$name:x"` with certain letters after the colon changes the
  value instead of adding a colon (`"$R:app/x"` becomes the absolute path of
  `$R` followed by `pp/x`), and with its errors discarded such a command just
  prints nothing, which reads as "nothing found". Such a command is now refused
  with the corrected form — in an agent's own commands, the reviewer's
  included, in commands you approve, and in the session terminal.
- **A Context Library replace no longer silently deletes another session's
  lines.** When an agent rewrites a whole file and its new version leaves out
  lines another session (or you) added since that agent last wrote the whole
  file, the write is refused and the lines are listed with who wrote them. The
  agent keeps them, changes only its own part, or confirms the deletion. This
  is the shared EOD file's case, where one session's redraft removed the other
  session's half.
- **Session documents stay usable when they grow.** Five changes to the
  document tools agents use:
  - A document can be corrected in place (`session_doc_edit`), instead of
    being rewritten whole or having a correction appended under the wrong
    text. The version before the edit is kept.
  - Listing a session's documents no longer returns every full body: a reply
    carries at most 48 KB, and a larger document is listed with its size and
    its headings.
  - One section of a document can be read by its heading, or a range of
    lines; a document too large to return whole answers with its outline.
  - A document's body can come from a file, so an agent can give the reviewer
    the test output it cites without re-typing it.
  - A document can be copied to the Context Library exactly as written, which
    is how a reviewer's handoff or an approved draft outlives the session.
- **A bot-hq session can close the feedback queue's rows, with the reason.**
  Rows agents filed with `file_feedback` could only be marked from the
  Feedback panel, so a session that shipped a fix could not close its row and
  stale rows piled up (27 at the last count, most of them long done). A new
  tool, `resolve_feedback`, closes a row as done (naming the commit) or
  dismissed (naming whose decision), from a session on the bot-hq project
  only; the panel shows the reason under the row and who closed it, named
  from that session's roster. Moving a row with the panel's buttons clears
  its reason, so a reason never describes a closure it was not written for.
  Migration 0101 adds the two columns.

### Changed

- **A policy can let file content name what a commit message may not.** One
  list, "Forbidden words in commits", screened both the message and the lines
  a commit adds, so a project inheriting a vendor's name from the general
  policy could not commit a comment citing `CLAUDE.md` or a path under
  `~/.claude-acct-2` (feedback #103). A policy may now carry a second list,
  "Forbidden words in file content" (`forbidden_in_content`, in Settings →
  Policy and in the YAML), that the pre-commit hook uses for the added lines
  instead — set it to just the attribution trailer in the general policy and
  the trailer stays out of source while the filename passes; the message scan
  is unchanged. Nothing changes until you set it. A project that carries its
  own message list keeps screening its content with that list unless it sets
  a content list too, so a general content list never loosens a project's
  own words. The agents' policy block says which words the content scan
  refuses, including for a policy with a content list only.
- **An agent's credentials are explicit.** Every agent, and the one-shot
  summarizer and "Test connection" probe, now starts with the inherited
  auth and billing variables cleared (`ANTHROPIC_API_KEY`,
  `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_BASE_URL`, the OAuth token variables, the
  Bedrock / Vertex / Foundry switches, `CLAUDE_CONFIG_DIR`) before bot-hq sets
  its own. A key exported in the shell that launched bot-hq used to reach the
  agents and bill it silently. A gateway model's own token and URL are set
  after the clearing and are unaffected.
- **The file viewer follows a participant to its folder.** Tool results of a
  participant on a second account live under that account's folder; the
  viewer now looks there.
- **A Tool Gate keyword that is only text no longer parks.** Keywords were
  matched anywhere in a command line, so a `grep` for "gh api" in your notes,
  or an `echo` that mentions `rm -rf`, waited for your approval like the
  command itself. A keyword appearing only as text given to a read-and-print
  tool (grep, echo, printf, cat, head, tail, wc, cut, tr, jq, diff, ls, ps
  and a few more), on a line that runs nothing else and writes no file, no
  longer parks. Everything else matches as before: a line with a shell, an
  interpreter, `$(…)`, a heredoc, any other tool or a file write keeps the
  whole-line match. Keywords now also catch other spellings of the command
  they name: `rm -fr` for `rm -rf`, and `git clean -df` for `git clean -f`.

- **`terminal_read` and `terminal_exec` return the terminal's text as the
  screen shows it.** They returned the raw bytes: colour and mode codes, the
  shell's prompt redraws, and its echo of a typed line (`p` + backspace +
  `python3`), and agents pasted that into chat as evidence. Escape codes are
  now removed, and carriage returns, backspaces and line erases are applied.
  `terminal_read` takes `raw: true` for the bytes as they came.

- **An agent's core bot-hq tools are ready from its first turn.** The turn
  verbs, the session-document tools, the Context Library readers, the gates
  and the commit checks no longer need a lookup before their first use, at
  the start of a session or after a compaction. The less common tools still
  load on demand.
- **The "user hasn't heard from you" reminder is off for agents.** It made
  agents post short status lines mid-turn that you, and then the other
  participants, had to read past. The line under the message box now shows
  what the working participant is running instead, in the tool call's own
  words, with how long it has run. Launch bot-hq with
  `CLAUDE_CODE_SILENT_TURN_REMINDER` set to keep the reminder.
- **The context warning comes while the handoff can still be written, and
  repeats after a compaction.** The rows came at 85 % and 95 % of the window,
  once per session. claude-code compacts 33,000 tokens short of the window
  (96.7 % of 1M, 83.5 % of 200k), so on a 200k window neither row ever came
  before the compaction, and on 1M one long turn often ran from under 95 %
  straight past it. The rows are now timed from that point: the first about
  6 % of the window before it (907,000 of 1M, 142,000 of 200k), a last call
  about 2 % before it (947,000, 152,000). They appear each time the context
  fills again, and say how far off the compaction is and when the
  participant's handoff document was last written.

### Fixed

- **A `gh api` read whose endpoint holds a shell variable no longer summons
  the reviewer.** `gh api "repos/o/r/pulls/$n"` — a GET with the number in a
  variable, inside a loop or not — was classed as an outward publish and
  queued for the reviewer's read before it could park for you, although
  nothing in it publishes. A double-quoted endpoint with a literal start is
  now read as the GET it is (the quotes keep it one argument whatever the
  variable holds); it still parks for your approval like any other gated
  `gh api`, and the no-approval read path still refuses every computed value.
  An unquoted variable, a computed flag value or body, or a start that could
  become `graphql` or a flag stays outward.
- **Two more provider refusals pause the participant instead of halting the
  session.** `Your organization has disabled Claude subscription access for
  Claude Code · Use an Anthropic API key instead …` (seen 2026-10-14) and the
  `402 … would exceed your available credits given your current in-flight
  requests` error (2026-10-17) were not in the limit detector: both
  participants erred one after the other and the session halted on
  "back-to-back errored turns". They are now read as limits with no reset
  time — the participant pauses on you at the first sighting, the account
  and model are marked, and the peers are told not to take over its work.
- **claude-code's own limit lines pause the participant, whatever their
  wording.** A session on Fable ended on `You've reached your Fable limit.
  Switch to another model, or manage usage credits at … to continue.`
  (2026-10-18): the detector did not know the line, so the session halted on
  back-to-back errored turns and no limit mark was set. claude-code's own
  messages are now read against its own list of limit openings — the
  included Fable allowance, a model that requires usage credits, the monthly
  spend limit, the team's shared budget, the organisation and seat-type
  refusals — and against the tail it puts on every such line, so the
  participant pauses on you at the first sighting and the account and model
  are marked (no reset time is named, so the mark says since when). Only
  claude-code's own messages are read this way: an agent writing the same
  words is still just talking, and the CLI's warnings ("You've used …",
  "You're close to …") are not limits.
- **The close card reads as a close, and the strips above the input box
  stay in bounds.** The card an agent parks when it thinks the work is done
  was dressed as a generic "APPROVAL · is blocked until you answer" with
  Approve/Reject buttons, although Approve closes and archives the session
  and the typed text becomes the agent's next instruction. It now says
  "CLOSE SESSION?", its buttons are "Close session" and "Keep open", and the
  text box says what the words are for. A gate's request — a close recap can
  run to a thousand characters — is clamped to four lines with a "show the
  full request" toggle (expanded, it scrolls in its own box), so it no longer
  squeezed the chat to nothing; the header row and the scheduled-run rows
  wrap on a narrow pane instead of pushing their buttons out of the box. A
  scheduled run's Cancel asks first ("Cancel this scheduled run?") and shows
  the command it withdraws. "View <file>" from Approval details closes the
  details first, so the viewer no longer opens behind it with one Escape
  closing both.
- **Windows: no console window beside the app.** The Windows build was a
  console program, so a black console window came up with bot-hq, and closing
  it quit the app and every running agent with it. The release build is now a
  windowed program: only the bot-hq window opens, and every program it starts
  (claude, git, Git for Windows' shell, gh) runs without a console window of
  its own. The git hooks and the Tool Gate, which run bot-hq's own `.exe`,
  still read and print through the pipes they are given, and print to your
  terminal when you run them there. Every release build now checks the pipes
  on Windows before it is published; the terminal is one of the checks in
  `docs/WINDOWS-TESTING.md`, made on a Windows machine. A debug build stays a
  console program, so `cargo run` keeps its log in the terminal.
- **Switching away from unsaved edits asks first.** Picking another role in
  Settings → Roles, opening another file in a session's Context tab, or
  closing the session settings drawer threw away what you had typed without a
  word — the editor simply remounted. Each now asks "Discard changes?", with
  Cancel keeping your edits. The drawer closes three ways — its close button,
  Escape and a click outside it — and all three ask; a click outside it now
  closes it at all, because the page behind is dimmed as the dialog it is.
- **Keyboard focus is visible everywhere, and motion respects the system
  setting.** Only the main button style showed a focus ring; a session tile,
  a tab, a close button or a "View file" button took focus with no sign of
  it. Every focusable control now shows a one-pixel ring when reached from
  the keyboard (a mouse click leaves none); the full-bleed editor and the
  terminal are left alone. The bare "×" buttons in the New Session and Model
  dialogs are proper 28 px icon buttons. A reduced-motion setting stops the
  pulses and spinners.
- **A usage limit reads as a warning and says since when.** The limit line
  under a model row and beside a New Session participant was muted grey like
  a hint; it is in the warning colour while the limit stands. A mark with no
  reset time ("You're out of usage credits") said only "no reset time
  given"; it says since when it has stood ("limited since 12:08 PM"), and a
  reset is given to the minute without seconds and with the date only when
  it is not today. The Models list's Clear button reports a failure instead
  of failing silently. The New Session dialog says why Create is greyed out
  ("Give the session a title", "Pick a role for every participant") instead
  of leaving you to find the gap, and the first-run tip names the shortcut
  your machine uses (Ctrl+N off macOS). One module now holds the
  account-and-limit rules the list and the dialog had each copied.
- **Names instead of identifiers, times that keep counting, copy that points
  at real controls.** The notification bell listed "Session s-69ef19"; it
  lists the session's title. A tray card's kind read as its tool's name
  (CHOICE, REQUEST); it reads "Question", "Approval request". Four plugin
  error banners that started with an internal error kind are the one error
  banner the rest of the app uses. "x ago" labels on chat rows and session
  tiles froze at whatever they said when they rendered ("just now" for an
  hour in an idle session); they tick every 30 seconds on one shared clock
  and show the exact local time on hover, and a tile no longer prints its age
  twice. The bare "✕" in the session header, which ends the agents and
  archives the session, is a labelled "Close session…" button. Two help texts
  dropped notes about bot-hq's own development history; the Context
  Library's empty state pointed at a disabled Rescan and the Promote dialog
  at a button that does not exist — both name the Register button. The
  diagnostics consent wraps instead of cutting off its "What is sent" link,
  and a session tile's chip row wraps instead of clipping its age at three
  columns. The bell no longer pulses whole for hours — only its count does,
  and not under a reduced-motion setting.
- **Screens say "loading" while they load, and an error reads as text.** A
  cold open of a session flashed "Session not found." until the row arrived;
  the Feedback panel, the document tabs, the tray tab, the Roles pane and the
  Context tab showed their empty-state copy during the fetch and after a
  failed one. Each now shows a placeholder while loading, the error when the
  read failed, and the empty copy only when the answer was empty. Four
  places that caught an error from bot-hq (cancelling a scheduled run,
  dismissing an action item, a Diagnostics endpoint call, opening the
  Terminal tab) printed `[object Object]`; they print the message. The
  Feedback panel names who filed a row as `ROLE · Model` from that session's
  roster instead of an internal name, and its filters are labelled. The
  confirm dialog's body scrolls, so a long plugin install consent no longer
  pushes its buttons off the window. The Settings tab row wraps instead of
  clipping its last tabs on a narrow window (and `?tab=notifications` /
  `?tab=diagnostics` now open those tabs), and the window has a minimum size
  of 1024 × 540.
- **An agent is reminded to write its handoff mid-turn, not only between
  turns.** The context warnings were posted when a turn ended, so an agent
  whose context filled inside one long turn — most of the executor's did, in
  the two sessions measured — compacted without ever reading one. bot-hq now
  reads each API call's prompt size as the turn runs; when it passes a warning
  point and the agent's handoff document has not been written since, every
  tool call the agent makes carries a one-paragraph `[bot-hq]` reminder with
  the figures and the document's name, until the agent writes it. A
  compaction clears the reminder. The reminder is context only — it never
  allows or refuses a tool call; the Tool Gate and the reviewer's deny list
  decide those exactly as before.
- **A document an agent leaves for you to act on survives the session.** The
  end-of-day message, a time-tracker entry, a follow-up note — drafts you post
  under your own name — lived in session documents that were archived with the
  session, so an agent copied them into the project's handoff file by hand to
  keep them reachable. An agent can now write such a document with
  `kind: "user_action"`: its tab is badged FOR YOU, and the dashboard shows a
  **Your action items** panel listing every such document from every session,
  open or closed, with its first line, Open (which lands on the document's
  tab) and Dismiss. Dismissed items leave the list and the document stays;
  an agent that rewrites one lists it again. No review step in this first
  slice — that is the second half of the suggestion, still to decide.
- **Approve a command any time; it runs inside its window.** Some commands may
  run only in a slot — a production merge in the six minutes a deploy cannot
  hurt anything — and until now the agent waited for the slot in a shell loop,
  parked the gate as it opened, and your approval raced the clock (on
  2026-10-01 a merge approved one minute late was refused and waited another
  hour). An agent can now park a command with a run window
  (`action_gate(run_not_before, run_not_after)`; the end is required and at
  most 24 hours ahead). The card shows the window; you approve whenever you
  see it. Approved before the window opens, the command is scheduled: a
  "⏱ Scheduled" line above the input shows when it runs, in how long, with a
  Cancel that takes the approval back, and bot-hq runs it at the start,
  unattended, posting the output as a `[system]` row linked to the gate.
  Approved inside the window it runs at once; after the window closes it is
  refused and says so. A relaunch loses nothing — the due runs are read from
  the table. `gate_status` reports "scheduled" until the run. A command runs
  once and only once: the run is claimed before it starts, so the scheduler
  never runs a command that an approval inside the window is already running,
  and a run that bot-hq was quit in the middle of is reported at the next
  start — not run again, since whether it did its work is unknown. Closing a
  session cancels its scheduled runs (nobody would be left to read the
  output); the close card says which, so you can reject the close instead.
- **An urgent gate finds you instead of waiting to be noticed.** On 2026-10-01
  an agent parked the cancel and the rollback for a job overrunning a shared
  API budget at 06:23Z; you approved them at 08:18Z, because nothing told you
  they were waiting. An agent can now mark a gate urgent with a one-line
  reason (`action_gate(urgent_reason)`). The command parks whatever the Tool
  Gate list says, the OS notification fires at once even while bot-hq is the
  focused window, and it repeats every ten minutes until you answer — at most
  six times. The session's tile turns red and names the urgent approval, the
  card leads with the reason, and a `🚨` row in the chat records who marked it
  and why. A relaunch with an urgent gate still parked notifies again. No
  phone push — this is the desktop tier you picked — and the Settings
  notifications toggle still silences everything.
- **The reviewer's turn opens with what its peer's prose could not show it.**
  A reviewer receives only what the other participants write in the channel;
  their tool calls never reach it, so a Context Library write or a session-doc
  write was invisible to review, and a gate you answered while the reviewer was
  working was a state it could later contradict. A participant holding the
  finding capability now gets a short `[bot-hq]` digest at the start of each
  dealt turn — its peers' (and your) CL writes and session-doc changes, and the
  gated commands you answered, each with its time, writer, verdict and first
  output line — in front of the rows it reads, never as a chat row: nobody
  else receives it. The newest 30 items within 6,000 bytes are listed and
  earlier ones counted; a gate still running when listed is listed again when
  it finishes.
- **The reviewer reads the close recap before you see the close card.** An
  agent's `close_session` used to park the card for you at once, so an
  advisory the reviewer would have raised, or a correction it had not relayed
  yet, raced your Approve. With a reviewer in the session the close is now
  queued for its read first — the recap is posted to it and it is summoned —
  and the card parks for you only after the reviewer's turn ends with the
  recap read; a blocking finding naming the close keeps the session open
  instead, and an advisory naming it is a note — it never holds the close the
  way it holds a publish for a revision. A session with no reviewer parks the
  card as before, and so does one whose reviewer is stalled or dead: the
  agent's reply then says the recap was not read.
- **A gate id in the chat shows the gate's state beside it.** An agent's
  "waiting on your approval of `236473a7`" used to stand after you had already
  approved it. An 8-character gate id quoted in a chat message now carries a
  small chip — pending, approved, rejected or withdrawn — read from this
  session's gates, so a stale status line is contradicted where it is read.
  Only a token that is one of this session's gates gets a chip; a short commit
  sha, which shares the shape, does not.
- **A tray question that names a session document links to it.** When an
  agent parks a question and writes the document's name in backticks
  ("Approve the key questions? They are in `key-questions`"), the tray card
  now shows the named documents as links that open their tab — a phase doc's
  or a custom one's — so you can read what you are approving without hunting
  for the right tab. No new tool parameter: the name in the prose is the link.
- **The reviewer can ask for a revision of a queued publish without blocking
  it.** An advisory finding that names a queued outward publish now holds it
  queued — not withdrawn, not parked for you — until the executor changes the
  body and re-issues the same command; the new body is re-posted for the
  reviewer's read, the advisory is settled as fixed with that on the record,
  and the publish parks for you once read. One approval publishes the
  corrected text instead of a publish and a second gated edit; blocking
  findings stay reserved for real defects. A re-issue with the body unchanged
  re-posts nothing.
- **A gated command that runs a script approves the script as it is, not the
  path.** `bash /tmp/check.sh` used to show you one line and run whatever the
  file held when you approved. The scripts a command runs (`bash X`, `sh X`,
  `python3 X`, `source X`, `./X` and the like) are now hashed when the gate
  parks — the card names them with their hash — and re-hashed at approval: a
  script changed in between is refused ("NOT RUN: the script changed after it
  was parked"; re-issue to pin the new version), and a pinned one that went
  missing is named. A script that could not be read when the gate parked — one
  the command itself creates, a path only the command's `cd` resolves — is not
  pinned: the card says so and it runs as it is, with a note. `~/x` means your
  home. Inline `-c` text, modules and stdin scripts are not files and are not
  pinned.
- **A session's gates have a ledger.** `gate_history` lists the session's
  gated commands newest first — each one's short id, time, issuer, state
  (queued for the reviewer, pending your approval, approved with its exit code,
  run time and output size, rejected with your words, withdrawn) and the
  command, with secrets redacted — so a participant whose context was
  compacted can reuse an approved command shape verbatim instead of parsing
  its transcript from disk, and "did that run?" has one place to look.
  Read-only; `gate_status` still gives one gate's output.
- **The close card names what would archive unnoticed.** Besides the open
  advisory findings, the recap now lists the open BLOCKING findings (the
  commit gate holds them against a commit, not against a close) and the custom
  session documents never promoted to the Context Library, or changed since
  they were — approved content that lives only in a session doc, such as the
  key questions that once had to be copied by hand at close — with the one
  call that carries a doc over byte for byte. A doc promoted with `cl_write_file(session_doc:)` is
  remembered as promoted, and where to. Handoff documents are left out on
  purpose: they are per-session by your decision.
- **Renaming or deleting a project takes its search index with it, and waits
  for its sessions.** The Context Library's retrieval index (`cl_atoms`) has no
  link to the project row, so a renamed project's atoms kept the old name and
  stopped answering, and a deleted project's stayed behind — 116 rows from two
  deleted projects were still answering `cl_retrieve`. A rename now repoints
  them and a delete removes them, orphans are swept at startup, and a project
  with an open session is neither renamed nor deleted until that session is
  closed (the refusal names it).
- **An agent's rewrite can no longer bury what you typed into a session
  document.** When you answer inside a document in its tab and a participant
  then replaces that document without having read your version, the replace
  is now refused, naming you (or the peer) and the time of the change; the
  participant reads the document whole and writes again, or appends, or edits
  only its part. Every document records who last changed it. (The twelve
  answers you wrote into `decisions-needed` on 2026-10-12 vanished this way —
  into the archive, where they were found later.)
- **A write to focus.md says what the file has become.** The reply to any
  `cl_write_file` of a project's `focus.md` now ends with its size, its
  line-start `SUPERSEDES:` count and how many appends it has had since its
  last replace, and — past the same thresholds the spawn-time warning uses —
  the reorganize advice itself, at the moment something can be done about it
  rather than at the next spawn, where nothing can.
- **Agents are told their thinking is never shown.** Several short status
  texts a participant "wrote" never reached the chat or its peers; they were
  the model's reasoning, which bot-hq never posts (and claude-code's own
  transcript had no text for them either). The rules now say so — a status, a
  finding or an answer that exists only in reasoning was never said — and a
  redacted reasoning block (one with no text) no longer risks taking the real
  text beside it down with it.
- **Open advisory findings are listed in full, and one can be read whole.**
  `check_open_findings` named only a count of open advisories, and the close
  card showed the start of each one with no way to read the rest. The commit
  check now lists each open advisory with its id and full text (they still
  never block), the close recap no longer cuts them at 140 characters, and a
  new `get_finding(id)` returns one finding whole — its text, where it points,
  who filed it, its status and disposition. And the close card says when the
  participant that asked to close is still acting on the recap's detail, so an
  Approve is not mistaken for a safe one while it finishes — a marker, not a
  hold.
- **The close-out staleness sweep reports terms, not vocabulary.** It flagged
  263 lines at one close — "follow-up", "byte-identical", "re-read", "1st",
  found in other projects' notes — none of them a thing to fix. Now a plain
  hyphenated word and an ordinal count only when the old text backticked them
  (an underscore, two hyphens or a non-ordinal digit still make a term); a
  write to a root-level library file has its words looked for in the root
  files and the session's own project only, never in another project's
  library (a retired filename is still found anywhere); and each hit names
  the write that retired the term.
- **The staleness report no longer lists a whole library as missing for a
  folder git tracks nothing under.** A project whose folder sits inside a
  bigger repository that tracks no file there (a mods folder under
  `~/Projects`, say) used to get an empty corpus, so `cl_stale_refs` reported
  every identifier in its Context Library as gone. The report now refuses
  such a folder with a configuration message that names the repository it
  sits in and what to do (register that repository's own folder, or commit
  the project's files). A folder that IS tracked inside a bigger repository
  still gets its report.
- **The staleness report reads the whole repository, and says what it read.**
  It used to search only ten source extensions (Rust, TypeScript, SQL, TOML,
  JSON, YAML, …) — 2 of one project's 121 tracked files, and nothing of a
  Python, shell, PHP or Makefile project — so every identifier in those
  libraries came back as "missing". It now reads every tracked file except
  prose (`.md`, `.markdown`, `.mdx`, `.rst`, `.txt`, `.adoc`), skipping files
  over 2 MiB and files that are not text, and the report opens by saying how
  many files it searched and what it skipped. A bot-hq tool name
  (`cl_index_search`, say) is never reported as a symbol missing from another
  project's code. The "code moved since written" section's lines no longer
  run together.
- **A temporary halt wakes on time after the Mac sleeps.** Its timer used to
  be one sleep for the whole delay on a clock that stands still while the
  machine sleeps, so a 30-minute halt declared before the lid closed woke 30
  minutes after it reopened. The timer now waits a minute at a time and
  re-reads the wall clock, so a session wakes at most a minute late, and a
  halt that was replaced or released ends its timer early. The row posted
  when a wake is skipped behind an open approval gate no longer runs its
  words together.
- **A model the claude CLI does not know runs at the window the registry
  says.** For a model id that is not Claude's own (OpenRouter, DeepSeek, …)
  the CLI only ever used its own 200k default, whatever the model row's
  Context window said — `xiaomi/mimo-v2.6-pro` on a 1.1M row compacted four
  times in one afternoon (2026-10-07). The row's Context window now reaches
  the CLI at spawn (as `CLAUDE_CODE_MAX_CONTEXT_TOKENS`, which the CLI
  honours exactly for such ids and ignores for Claude ids), on both the
  executor's and the reviewer's spawn and in "Test connection". The DeepSeek
  V4 Pro row (1,000,000) moves off the 200k default with it. The channel
  notice and the Model dialog no longer recommend `modelOverrides` for a
  gateway model: mapping one onto a Claude id makes the CLI send that Claude
  model's request profile, and OpenRouter answers `400 Mid-conversation
  reasoning effort (configuration_update) is not supported` — the 1.0.8
  advice was right only for a Claude id newer than the CLI's catalog (the
  Fable case), which the notice now says, with what the mapping's key is. A
  gateway model with no Context window on its row is now called out once per
  spawn instead of silently running at the default.
- **A session on a gateway model survives a tool call the model got wrong.**
  Through OpenRouter, MiMo once answered with a tool call that had no id and
  no name; claude-code dutifully recorded a result for it, and from then on
  every request replayed the pair and the gateway refused the whole
  conversation (`400 messages[43]: tool messages must include a non-empty
  string tool_call_id`) until the session was halted. The gateway proxy now
  removes such a call before forwarding and turns its result into a note that
  the call did not run (after the message's remaining tool results, never
  leaving a message empty), so the next request goes through and the
  participant can issue the call again. The transcript is left as it was; a
  conversation already poisoned heals on its next request.
- **On Windows, the second account's setup commands are PowerShell.** The
  Model dialog's sign-in and share commands, and the command bot-hq names
  when it refuses to start a participant in a logged-out folder, were written
  for macOS/Linux shells and did not run on Windows. They now come from one
  place in the shell of your machine: on Windows, PowerShell 5.1 commands
  that put `CLAUDE_CONFIG_DIR` back afterwards (even if you press Ctrl+C) and
  share the folders as junctions. On every platform the share step never
  deletes anything, and running it again skips what is already linked. The
  dialog also checks the folder before it shows any command, so a `~` path
  shows the reason instead of a command that would sign in somewhere else.
- **The Add/Edit model dialog fits the window.** It had no height limit, and
  once the Claude config dir field and its setup commands were added it grew
  taller than the window: the title and Save were cut off and nothing
  scrolled. It now opens at the New session dialog's size, in two columns
  that scroll on their own, with Cancel, Save and any error always in view.
  A text selection dragged out of the dialog no longer closes it.
- **The composer's working line now shows what is running.** It reads a
  turn's age, its tool count and the running tool from the chat, but looked
  the rows up under the participant's slot rather than its name. In the app it
  read "0 tools" and named no running tool, and a turn's age counted from when
  the line appeared. Found in the first live check after the relaunch.
- **Opening a session's Terminal tab no longer types onto its command line.**
  The tab replays the terminal's history when it opens, and the terminal view
  answered every query a program had left in that history (a CLI asking for
  the cursor position or the background colour) a second time. The answers
  landed on the shell's command line as stray characters such as `1;2c`. The
  replay now sends nothing to the shell except keys you type meanwhile;
  queries that arrive live are still answered.
- **`terminal_exec` no longer types into a program that holds the terminal.**
  A command was written into the session terminal whether the shell was at
  its prompt or not, so a pager left open, a server started in the background
  of the visible terminal, or a password prompt received it as input. It is
  now refused, naming the program. An agent can still type a line into a
  program it started there on purpose (a Python prompt) by saying so
  (`to_job`), but never into a password prompt such as `sudo` or `ssh`, even
  one further along a pipeline (`cat x | sudo tee f`).
- **`terminal_exec` waits for a command to finish.** It returned at the first
  0.7 s of silence, so a command that starts slowly — a test run after a few
  quick `git` lines — came back without its results, and a second call was
  needed to read them. It now waits until the shell is back at its prompt (on
  macOS and Linux), and when a command still holds the terminal at the time
  limit, the note names it: a pager or a prompt waiting for input.
- **A reviewer writing down its findings no longer cancels the executor's
  phase vote.** Review notes in a `<phase>-eyes` document used to count as a
  change to the work, so the executor had to vote again each time, and
  reviewers kept findings in the chat to avoid that. The notes now withdraw
  only the reviewer's own vote, and a line in the chat says that notes landed.
- **A reviewer's document write can no longer land in the executor's phase
  document.** Writing to `plan-eyes` (or to `plan`) without naming the phase
  appended the review into the executor's plan while reporting that it had
  written `plan-eyes`. Such a write now always goes to the reviewer's own
  document, and an executor's write to a reviewer's document is refused.
- **A phase vote is refused when the state of the work cannot be read,**
  instead of being cast on an empty state that every other vote would match.
- **Agents are no longer told to add a co-author trailer your policy
  forbids.** The claude CLI instructs every agent to end commits with one;
  when your commit policy forbids it, bot-hq now switches the CLI's
  attribution off for that session, so the instruction never reaches the agent.
  A policy that does not forbid it changes nothing.
- **An agent's background helpers are no longer cancelled while another
  participant holds the turn.** A helper's messages arrived on its parent's
  output and read as the parent starting a turn of its own, which 1.0.8 stops
  while someone else holds the turn — and that stop cancelled every helper the
  parent had running. Helper messages are now recognised and kept out of the
  chat, and a self-started turn is not stopped while the agent still has
  background work running; a notice says why instead.
- **A helper's messages no longer appear in the chat as the agent's own words,**
  and no longer replace the agent's context-meter reading with the helper's.
- **A plugin sending to, or waiting on, a session that was closed gets a clear
  "closed" error** instead of "no live session", which a plugin could not tell
  from a passing failure. The session's history stays readable.
- **A participant that passes no longer marks its earlier phase votes as
  withdrawn.** Only the current round's ballots are retracted; the ones that
  carried an earlier phase change stay in the audit history as cast.
- **Context Library history says who changed what, and each entry holds only
  what it names.** Each agent write is now its own commit, named for the
  session and the kind of write (`cl: _globals/eod-tldr.md (hands, s-43584072,
  replace)`). Your own edits used to ride the next agent's snapshot under that
  agent's name; they now get a commit of their own first, and a save from the
  app's editors is committed as yours straight away. The reply to a write says
  which lines it changed. Two writes to the same file can no longer interleave
  between reading it and writing it.

### Security

- **Two development dependencies of the telemetry worker were updated**
  (Dependabot #7 `source-map-js` 1.2.1 → 1.2.2, #9 `undici` and `wrangler`).
  The worker under `packaging/telemetry-worker` is deployed on its own and is
  not part of the app; nothing bot-hq ships was affected.

### Deferred

- **Tailwind CSS 4** (Dependabot #8): a migration to its CSS-first
  configuration, not a version bump — its CI is red on all three platforms.
- **The design-system consolidation** the 1.0.9 UI survey found: one button
  face, one input and switch, one dialog shell, the participant palette and
  greys, the type scale, spacing and radius tokens, icons for glyphs, a
  Settings page wrapper and the chat's typography — a visual pass of its own.
- **Collapsing the "review notes written" notice** (one chat row per reviewer
  note, up to 30 a session): kept, because it is how the executor learns to
  read the notes.
- **Windows:** a Terminal-tab shell that has exited can still look alive
  (ConPTY delivers no end-of-file); `terminal_exec`'s screen-size reply with
  no Terminal tab mounted; child-process reaping through a Job Object. The
  console-window change is checked by CI over pipes only — the launch, the
  absence of console flashes and a hook printing in a terminal are the
  Windows-machine checks in `docs/WINDOWS-TESTING.md`.
- **glib 0.18** (RUSTSEC-2024-0429), pinned by Tauri's GTK3 stack on Linux —
  unchanged from 1.0.8.

## [1.0.8] — 2026-09-27

Stray turns, secrets in what bot-hq stores (F10), and two dependency
advisories (sessions s-02101415 and s-5482dfff). No migrations. F10: every
row an agent writes is stored with its secrets redacted; what you type stays
as you wrote it. Where and why: ARCHITECTURE.md §Secret redaction.

### Fixed

- **A second self-started turn in a row no longer takes an old turn
  number.** claude-code starts a turn by itself when a background task it
  ran finishes — a push that waited on your approval, for one. The first
  such turn, completing unbound, wiped the number the next one is checked
  against, so a second in a row bound itself to a retired turn and kept it
  through the ring's later deals; the halt that ended it was then reported
  as an error.
- **An errored turn's notice no longer says its text was lost.** Everything
  a turn writes is in the chat before the turn ends; the notice now says the
  turn ended in an error, quotes its last line, and points up at what it
  wrote.
- **A participant still orienting when boot times out stays out of the ring
  until it is ready.** Boot's timeout notice promised "the rest join the
  rotation as they finish", but the ring dealt them anyway: their
  orientation was cut short and reported as a turn while their real turn
  ran beside the next holder's. The ring now passes over them — in the
  rotation, for a mention, and for a typed message's interrupt — and deals
  them as soon as they report ready. When everyone else has voted done or
  passed, it waits for them with a notice instead of re-dealing a finished
  participant or yielding on a false "every participant passed".

### Changed

- **A self-started turn is stopped while another participant holds the
  turn.** A background task waking one participant mid-lap no longer leaves
  two working at once: the turn is stopped with a notice, and the
  participant acts on what the task reported at its next turn. A commit,
  push or migration in flight is never cut — the stop waits for it, and is
  skipped if it runs long. With nobody else busy, as when the session is
  waiting on you, nothing changes.

### Security

- **Everything an agent writes is stored with its secrets redacted, from now
  on**: chat messages, host notices, tray questions and options, session docs,
  findings, feedback reports, search queries, the violations log, halt reasons,
  text a plugin sends in, and agent writes to the Context Library. Each secret
  becomes `[redacted: <what>]`; rows stored before this stay as they were.
- **What you type is kept as written** — the composer, a staged message, a
  free-text tray answer or rejection reason, a custom doc you save, and your
  edits in the Context Library tab — since you may be handing an agent a token
  on purpose.
- **A gated command is kept as written**, as a recorded exception: it runs
  exactly as written, and the approval card shows exactly what will run.
- **An agent is told when its write was stored with markers** (a Context
  Library write or a session doc), and an edit that quotes a secret the file
  holds only as its marker is told to match the marker.
- **A secret a command printed is no longer stored in plain text.** Tool
  output and every string of a tool call's input are redacted before the
  row is stored — each self-identifying secret becomes `[redacted: <what>]`
  — and the row stays valid JSON. An approved command's output is redacted
  before it reaches the chat, so the agent that asked and its peers never
  receive the secret (a command the Tool Gate runs without asking returns
  its raw output to the agent, as a plain command does; only the stored copy
  is redacted).
- **A Laravel Sanctum API token is recognised as a secret** (the shape of
  the Laravel Cloud token printed on 2026-09-04), in redaction and in the
  Context Library push check. AWS's documentation example key is not.
- **An outward publish whose body carries a secret is refused**, whoever
  is reviewing — including a session with no reviewer and an approved
  reviewer override. Only published content is scanned; a secret in a
  request header is not.
- **rustls 0.23.45** — RUSTSEC-2026-0285 (TLS 1.3 handshake messages
  accepted across encryption levels). bot-hq is a TLS client only.
- **react-router 7.18.4** — an open redirect through a backslash in
  `<Link>`/`useNavigate`, and constructor injection in `deserializeErrors`.
  The app renders nothing on a server; `npm audit` now reports no runtime
  vulnerabilities.
- **The telemetry worker's deploy tooling** (wrangler, sharp and, through
  wrangler, miniflare) moves past three high advisories (Dependabot #6). The
  worker has development dependencies only, so nothing deployed changes.

### Deferred

- **glib 0.18** (RUSTSEC-2024-0429, an unsound `VariantStrIter`) is pinned
  by Tauri's GTK3 stack on Linux; Dependabot's security update fails on
  every run until Tauri moves to gtk-rs 0.20.

## [1.0.7] — 2026-09-25

Fixes for the accumulated agent-feedback queue (items #10–#45, sessions
s-84c59f27 and s-bdc7d2de). Four migrations apply at the next launch: 0083
(`findings.gate_id`), 0084 (`session_tray.body_sha256`), 0085
(`session_tray.run_refusal`) and 0086 (the Pause snapshot on
`cancel_events`).

### Fixed

- **A body only the executor had handled no longer counts as reviewed.**
  The outward-publish coverage check searched every row, including the
  executor's own tool rows, which no reviewer ever receives — a body merely
  written with a tool parked for your approval as if reviewed. Coverage now
  reads only the rows the reviewer's backlog delivered, clamped at the
  200 KB wire cut, and a publish that parks on prior review says so in the
  chat, naming the covering row (feedback #40).
- **A reviewer's blocking finding can name the publish it vetoes.**
  `flag_finding` takes an optional `gate_id`: a queued publish's id
  withdraws only that one, `"none"` withdraws none, and no id keeps the
  withdraw-everything default. Only open findings veto; a publish withdrawn
  by a finding is re-reviewed rather than re-parked on its old coverage;
  while a targeted finding is open, re-issuing the exact command is refused
  unless its body file changed; and the withdrawal notice no longer tells
  the executor to clear an unrelated finding first (feedback #42/#43).
- **Every outward body form reaches the reviewer.** Release notes
  (`--notes-file`, `--notes`), `gh api` fields read from a file, curl data,
  gist files, `-b` and `--body=` are extracted per `gh` subcommand and for
  curl; bodies that cannot be read up front — computed by the shell, read
  from stdin, written by the same command, `pr create --fill`, templates,
  unknown file flags — are refused instead of parking as content-free. The
  outward check also sees through env prefixes, `env`/`command`/`sudo`,
  `sh -c`/`eval`, scripts fed to a shell, command substitution, grouping
  and wrappers such as `timeout`, `xargs` and `find -exec` (feedback #35).
- **A reviewer that goes down no longer strands queued publishes.** The
  down-reviewer refusal comes before "already queued", and approving
  `override_reviewer_block` releases queued publishes to you as their own
  cards marked unreviewed — except any the reviewer had vetoed, which are
  withdrawn. The override prompt now says it also waives publish review
  (feedback #32).
- **An approved publish whose body file changed after review does not run.**
  The body files are hashed when a publish parks or queues and re-hashed at
  your Approve; a change or a missing file refuses the run with a chat row,
  and `gate_status` reports "approved but NOT RUN" (feedback #22).
- **`gate_status` says RUNNING** while an approved command is still
  executing, instead of "output delivered" (feedback #15).
- **The approval card shows why a command was gated** — the matched keyword
  and its column, and a prior rejection — on the card face, including for
  queued publishes (feedback #29).
- **A blocked push says why.** A 30-minute approval timeout now says the
  card is still live and not to re-issue the push; a token mismatch names
  the build mismatch; only an unreachable app says "not running"
  (feedback #36).
- **Context Library writes keep a real rollback point.** Content a CL file
  held that never reached git is committed alone before a write replaces
  it; if that snapshot fails, the write is refused. "Nothing to commit" is
  decided by git, a failed commit is reported as a failure, library git
  ignores signing config and repo hooks, and writes are serialised
  (feedback #27/#28).
- **The close-out staleness sweep reports stale references, not
  vocabulary.** Words count as retired terms only when backticked or
  code-shaped; filenames and anchored paths count; CL files deleted or
  renamed during the session are swept for; files the session wrote are
  skipped for words only; a restored term is dropped (feedback #21/#38).
- **`cl_rescan` reports rows it found already current** (`unchanged`), and
  says the file watcher keeps the index current (feedback #20/#24).
- **A message sent while the agents are booting waits for boot to end, and
  READY comes first.** Staged or typed, a plugin's first prompt, or Resume —
  anything that arrives during boot is held until every participant has
  oriented, instead of dealing a turn nothing could complete. READY is posted
  before the message box unlocks and before the held message goes out, and it
  says the message goes out now rather than asking for a task. A typed Send
  during boot no longer interrupts the agents' orientation, and boot counts
  each participant once, so one that answers twice cannot end boot early
  (feedback #10).
- **A merge commit is screened for its own lines only.** While a merge is in
  progress, the pre-commit content scan checks only the lines new to every
  parent — conflict resolutions and new text — so a forbidden word someone
  already committed on the other branch no longer forces `--no-verify`; the
  post-commit check follows the same rule. The commit-message check is
  unchanged, and a configured external diff tool can no longer blank the scan
  (feedback #39).
- **Dashboard cards and the notification bell show every kind of wait.**
  An approval waiting, a halted session ("halted — your move", with its
  reason) and questions are each named on their own, where before only
  questions showed; a temporary halt shows a muted "wakes 14:05" until its
  wake time passes.
- **Dashboard cards can now be swapped.** Dragging one card onto another
  swaps them — the drag now runs on pointer events, since the app window
  never delivered the browser's own drop event — and a plain click still
  opens the session.
- **A participant colour you pick is yours alone.** Picked hues leave the
  rotation, the new-session dialog marks a colour another participant picked
  as taken, and two participants that would display the same name are
  numbered at creation — "Reviewer", "Reviewer 2" (feedback #11/#12).
- **Error halts give advice for the error they saw.** Only a context
  overflow advises a fresh session; an upstream failure says the retries
  are spent; an unknown cause says so. A failed turn with no text reports
  the result's subtype and HTTP status, a refused connection is retried,
  and the banner counts the participant's error halts and its last clean
  turn (feedback #17/#18/#19).
- **claude-code's own error text is a notice, not the agent speaking**, and
  so are the retry supervisor's notes (feedback #17).

### Added

- **Custom session docs keep their earlier versions**: a replace archives
  the previous body as `slug@n` (the newest 10 kept), and
  `session_doc_read` takes `grep` and `lines` for a selective read; archives
  stay out of an agent's `session_doc_search` unless asked for
  (feedback #37).
- **A run of passes is one compact chat line** — the `pass_turn` call and
  result rows are hidden and consecutive passes read "passed — A · B"
  (feedback #13).
- **The starter Tool Gate gates history rewrites** (`git filter-branch`,
  `git filter-repo`, `git reflog expire`, `git gc --prune`).
- **A long turn says it is working.** The composer's status line reads
  "HANDS is working · 12m · 37 tools" (taken from the chat, so it survives
  a reload mid-turn), a staged message shows that it lands when the turn
  ends, and a turn past twenty minutes posts one row saying so and that
  Pause interrupts it (feedback #44/#45).
- **A Context Library write warns when another session wrote the same file
  in the last fifteen minutes** (feedback #44).
- **Pause records what the turn was doing.** Pressing Pause posts a row
  such as "Paused while hands held the turn (23m 10s in): 1 tool running
  (Bash, started 4m 02s ago); last activity 3m 52s ago", and the cancel
  record keeps the same facts for later diagnosis (feedback #44).
- **The footer shows which build is running.** "bot-hq v1.0.6 · 9bb7e53"
  replaces the old copyright line, with the build profile, program file,
  build time, data folder and database version on hover. A "restart
  pending" chip appears when the program file changed since launch — the
  git hooks already run the new one — and "rebuild needed" when it is gone;
  "N working" counts the sessions a relaunch would interrupt.

### Changed

- **The Tool Gate settings copy states the snapshot rule**: a session
  copies the global list when it spawns (feedback #33/#34).
- **The universal rules teach the shell traps** that produced false checks:
  a native claude-code's `find`/`grep` wrappers, zsh's `"$r:path"` modifier,
  unquoted globs, and a pipeline's exit status (feedback #41, #27/#28).
- **The frontend builds with vite 8, vitest 5 and @vitejs/plugin-react 6**
  (supersedes Dependabot #4). The build target stays at the old browser floor,
  so older Linux webviews keep loading the app, and Node 22.12 is now the
  minimum.
- **Release builds stamp their commit**: `./start --release` and the release
  workflow pass it to the build for the footer; the signing guide now says to
  uncomment only the `APPLE_*` lines of the workflow's live `env` block.

## [1.0.6] — 2026-09-09

The first release built from the public repository,
[`gregoryerrl/bot-hq`](https://github.com/gregoryerrl/bot-hq). The
repository starts from a single commit; development history before it stays
in a private archive, and this changelog is the public record of it. No
feature changes — the release exists so that the public tree, the published
bundles and the update check all describe the same source.

### Changed

- **Two migration comments are worded neutrally**, and the app repairs the
  checksums of installs that applied the earlier wording. sqlx refuses a
  database whose stored digest for a migration differs from the one the
  build embeds, so a reworded comment alone would have stopped every
  existing install at launch; the repair that already brought Windows'
  CRLF-stamped databases forward now recognises the superseded digests too
  and rewrites them before the migrator runs. A digest it does not recognise
  still fails, as before.
- **The universal rules' halt-reason example**, a test fixture and two
  archived planning documents no longer name a specific hosting platform or
  project.

### Removed

- **Unused captured fixtures** under `docs/stream-json-samples/`;
  `docs/stream-json-events.md` documents the schema, and no test loaded them.

### Fixed

- **The release workflow attaches the bundles it built.** 1.0.5's run
  reported success with an empty draft release: the two-job shape
  introduced in 1.0.5 downloads every platform's artifact into one
  directory, where `upload-artifact` had rooted each at `target/`, so the
  attach step's flat globs matched nothing and `fail_on_unmatched_files`
  was off. The globs recurse now and an empty match fails the job. 1.0.5's
  four bundles were attached from the run's artifacts by hand, with their
  checksums recorded in the release notes.

### Security

- Bundles are unsigned, as before; the checksums in each release's notes
  are the only integrity signal. The in-app update check only notifies —
  it never downloads or installs — so verify a download against them
  before opening it.

## [1.0.5] — 2026-09-06

The hardening release. A security sweep of the repository and the running
app (2026-09-06) found no credential anywhere in the code or its history,
but it did find that the app's two loopback listeners trusted anything that
reached them. Everything below under **Security** closes that; the rest is
the week-35 friction work that was already queued, the Windows CI job
finally going required, and a handful of small fixes.

### Security

- **The signaling server refuses callers it did not spawn.** A session/agent
  pair with no secret registered used to be let through (an upgrade-
  continuity choice that no longer had a client), which meant any local
  process — or a web page that found the port — could invent a pair and
  reach the ungated tools, including the ones that click in the app's own
  window. It now refuses, and every route refuses browser-originated
  requests outright (the `Origin` / `Sec-Fetch-*` headers a browser cannot
  omit). Verified against the real claude-code client's request headers.
- **The hook routes need a per-launch secret.** The git pre-push and
  Tool-Gate hook subprocesses now present a token bot-hq mints at startup
  (`<data_dir>/.local/hook-token`, user-only); without it, the routes that
  park an approval card in your tray are refused — before, anything on the
  machine could park a card under an agent's name. The signaling address
  file is user-only now too. A hook binary older than the running app gets a
  401 whose text says which binary to rebuild.
- **The gateway proxy forwards only to gateways you configured.** The local
  proxy that normalizes requests for non-Anthropic gateways took its
  destination from the request itself; it now forwards only to an upstream a
  spawn resolved from your model settings, and refuses browser-shaped
  requests.
- **A plugin manifest cannot write outside its own directory.** On a URL
  install, `entry` is validated like every served path (no absolute path,
  `..`, backslash, drive prefix or percent-encoding), and the write site
  re-checks the joined path.
- **A content security policy on the main window**, and the web inspector
  is no longer compiled into release builds (`./start` keeps it for local
  runs). Verified live across the dashboard, a session, the terminal,
  settings and a plugin panel.
- **bot-hq's own git invocations ignore config-named programs.** The Apply
  diff, the untracked listing, worktree operations and the stale-refs scan
  run git with `core.fsmonitor`, hooks, external diff and textconv drivers
  disabled, so a repository delivered with a hostile `.git/config` runs
  nothing when a session merely opens it. (`.gitattributes` filters still
  run on checkout — disabling them would break Git LFS; a registered
  repository remains trusted code, see `SECURITY.md`.)
- **CI runs with read-only tokens and sha-pinned actions**; only the job
  that attaches release bundles holds `contents: write`; the Pages deploy
  skips itself while the repository is private.
- **The Context Library push scanner catches more shapes**: bare `sk-` keys,
  Google, Stripe, GitLab, Hugging Face and npm tokens, signed JWTs, an AWS
  secret key beside its config key, and the `.env.<stage>` / `.envrc`
  filenames it used to miss.
- **`SECURITY.md`**: how to report a vulnerability, and the trust boundary
  in plain terms.
- Dependencies: the one compiled `cargo audit` advisory (quick-xml, via
  plist) is closed; the non-breaking npm fixes are taken.

### Added

- **`cl_edit_file` — an in-place Context Library correction.** An agent
  that had to change three lines of a 38 KB file could only re-emit the
  whole file or append a correction 296 lines below the claim it replaced
  (feedback #30; ~60–70k output tokens spent that way in one session). The
  new tool replaces `expect_occurrences` (default 1) exact occurrences of
  `old_string` with `new_string` in an existing file, refusing and naming
  the count when it differs, behind every `cl_write_file` guard: traversal,
  the 1 MiB cap on the result, the >50 %-shrink refusal without
  `confirm_shrink`, protected `_globals` and agent-invisible files, the
  status-flip lint, the retired-term diff, the atomic write, the git
  snapshot (named in the reply), the index rescan, the close-out gate lift
  and the private-remote push. Same capability as `cl_write_file`
  (`write_context_library`); the parity oracle records it as a sanctioned
  divergence the user chose. `cl_write_file` also accepts `content_path` —
  an absolute path whose UTF-8 text is the body, capped at 1 MiB by stat
  before it is read — so a body built on disk is handed over instead of
  re-typed. The general rules, the tool descriptions and the close-out
  prompt now say: append for new sections, edit for corrections, a full
  replacement only for a restructure.

- **A participant's context passing 85 % and 95 % is announced in the
  channel.** Four of the seven dissected sessions ran the executor past
  92 % of its window with nothing in the channel saying so — the header
  pill is not something the user is necessarily watching, and "can you
  compact?" is not something bot-hq can act on. The pump now posts one
  system row per band ("⚠ hands's context is at 90 % (900000 of 1000000
  tokens) as of its last turn — the point to hand off to a successor
  session is near"), a state line about the named participant rather than
  an instruction to whoever is mid-turn. The latch ratchets and never
  lowers: an auto-compaction drops the meter but does not re-arm the row,
  so a session hovering at 95 % is told once, not every turn; a respawn
  starts over. A zero or missing window says nothing. The header pill's
  own colours (warn at 70 %, critical at 90 %) are unchanged.

### Changed

- **An outward publish the reviewer has not read is queued, not refused.**
  The 1.0.3 outward-review precondition refused an `action_gate` park on a
  `gh` / `curl` command until the reviewer had been dealt a turn and had read
  the exact body — teaching a two-turn ritual ("end your turn, then park on
  your next"). Measured over the seven client-project sessions of Aug 31 – Sep 5:
  83 refused attempts, one deadlock (#32 — each typed user message restarted
  the rotation at the executor, so the reviewer was never dealt), three idle
  nudges caused by the ritual, and a 25 KB issue body re-emitted into a
  session doc to count as delivered. A refused park is now a durable tray row
  with `status = 'queued'` (`session_tray.body_row_id`, migration 0080):
  bot-hq posts the body to the channel for the reviewer, summons them for the
  next turn, and flips the row to the user's `pending` card only after the
  reviewer's cursor has passed that body row — a blocking finding filed in
  between withdraws it instead, with a system row saying so. The same gate id
  carries through; `gate_status` reports `queued`. A queued row latches
  nothing and renders nowhere (the tray read carries an explicit status
  allow-list), counts as pending for the idle watchdog, is deduplicated on an
  identical re-issue, is re-summoned when a ring registers after a relaunch,
  and is named in the channel and withdrawn at close. Correctness refusals —
  reviewer down, empty or unextractable body — still refuse. The executor may
  halt while a publish is queued; the "end your turn bare" instruction is
  gone from the hook's and the tool's text, though the general rules keep the
  older ritual as a fallback until the mechanism is proven live.

- **An agent's `close_session` parks a close card instead of closing.** The
  tool was gated on the capability alone; "once the user approves the close"
  was prose, and two closes in the week of Aug 31 had no approval — one
  re-used an "if all green, reclose" given before a reopen ("reopening again,
  please don't close without my permission"), the other fired 90 seconds
  after "before you close, draft me a message". Now an agent close needs a
  user Approve on a close card given since the session was last (re)opened
  (`sessions.reopened_at`, migration 0081, compared with
  `COALESCE(reopened_at, created_at)`); otherwise it parks a `close` card in
  the gate slot — Approve closes through the ordinary close path (the
  close-out epilogue unchanged), Reject keeps the session open and tells the
  executor to keep working, the typed answer being the next instruction. The
  three close nudges collapse into that card's recap: the staleness sweep is
  demoted to a one-line advisory (every hit measured this week was
  vocabulary), the open-advisory count rides along, and the learnings-delta
  nudge stays as the one refusal on a session that wrote no CL delta — a
  close now costs at most two tool calls and one click instead of four
  calls. The UI Close button and who holds `close_session` are untouched.
  **If you have edited the HANDS role prose**, note that the matching prose
  change (migration 0082 — "call `close_session`; it parks the card") only
  reseeds a row that still carries a shipped seed byte-for-byte, so your
  customized row keeps the older "park `ask_user_choice("Close session?")`,
  then close" ritual and a close costs two clicks until you press reset to
  the shipped default in Settings → Roles (the tab already shows "Differs
  from the shipped default" on such a row) or paste the new paragraph in.

- **A reopen keeps the IPAV phase** instead of resetting it to nothing. The
  1.0.0 reset assumed a reopen "almost always starts a new task"; five of
  the six reopens in the dissected week came within ten minutes of the close
  and continued the same work, the roster never re-voted its way back, and
  the executor drew the "editing before Apply" nag on legitimate
  continuation edits. Recorded in the project CL's decisions log.
- **Every participant's prompt names its bot-hq session id** (in the
  participants section, with the note that the claude session id is a
  different string) — five CL clip files had cited a claude id prefix as the
  session because the id lived only in an environment variable.
- **`cl_write_file` reports its snapshot**: the reply carries the short sha
  of the library commit it took (the rollback point to record), says "no new
  snapshot" when the content was already there, and says "NO SNAPSHOT TAKEN"
  when versioning failed — a session had recorded a stale `git log` commit
  as its rollback point before a full-file replace because a write had
  never been snapshotted and nothing said so. Library pushes are serialised
  so two writes in one turn no longer race the remote ref.
- **Tool Gate refusals and approval cards name the keyword that matched and
  the column it hit**, so a destructive literal inside a grep pattern (a
  read-only verification) is recognisable as a data-string match at a
  glance — the match itself is unchanged. A re-parked command the user
  rejected earlier now carries that verdict on its card instead of reading
  as a first ask.
- **`cl_index_search` rows are slimmer and say how big each file is.** The
  boot call every participant makes was the one constant in the week's boot
  context (25–28 KB, 14 of 14 boots, ~490 bytes a row). Each row now carries
  `project`, `file_path`, `description` (capped at 160 characters), `bytes`
  (the file's current on-disk size, from a stat at call time — absent when
  the file is gone) and `updated_at` cut to whole seconds; `tags` only when
  the file has any; and `abs_path` only when the file is NOT at
  `<library>/projects/<project>/<file_path>` — every root-level `_globals`
  row and any project with a custom library path — so the resolved location
  agents once constructed wrongly is still handed over exactly where it
  differs. No row is dropped: the prompt's CL primer is 12 rows with 100-
  character descriptions and no `_globals`, so the boot call is not
  redundant with it. The tool description and the BOOT text now steer
  toward `cl_retrieve` for anything not needed end to end, stated as a
  bytes-to-tokens conversion rather than a threshold.

- The GitHub repository is private as of this release (0 stars, 0 forks, 0
  watchers at the time). While it is, the Homebrew cask cannot download the
  DMG — install from the release page with `gh release download` — and the
  Pages site is down. Client engagements once named in eight files carry
  neutral labels now; two applied migrations (0039, 0080) keep a slug each
  in a comment, since applied migrations are immutable.

### Fixed

- **An approved gated command that is still executing no longer draws an
  idle nudge** — the watchdog counts it as pending until its output lands.
- **A discarded errored turn is announced** with one system row (who, the
  last error line, that the ring moved on) instead of persisting the error
  text as if the agent had said it and silently dropping whatever the turn
  was asked to do.
- **A model the claude CLI does not recognise no longer runs silently at a
  200k window.** A saved model can now carry "Claude CLI settings" (Settings
  → Models; `models.cli_settings`, migration 0079) — a JSON object bot-hq
  merges into every participant's `--settings` at spawn, reviewer included
  (the read-only posture never received a `--settings` argument before). Its
  first use is `modelOverrides`: the CLI takes a model's context window from
  its own catalog, so `claude-fable-5-1` on CLI 2.1.251 ran 40 minutes at
  200k and auto-compacted three times while the header showed 1M
  (2026-09-03). Mapping a model id the CLI knows to the new one restores the
  real window; a role's own Claude-config override still wins on any key both
  set, and a row that is not a JSON object is refused at save and ignored at
  spawn so the Tool-Gate hook riding the same argument can never be lost.
  Companion: the pump now compares the CLI's reported window with the
  registry's and posts one channel notice naming both numbers and the fix
  when they disagree by more than 20 %, and the CLI's own
  `unrecognized_model` stderr line is logged at WARN.

- **The Windows CI job is required.** Its only failures were the five
  ConPTY-bound terminal tests (now ignored on Windows by name, with the
  reason in the attribute) and two tray tests racing a 20 ms sleep (now
  awaiting the call they were waiting for) — so a red Windows job means
  something.
- **A chat link to a claude-code tool result opens in the viewer.** The
  files claude-code spills to `~/.claude/projects/…/tool-results/` for this
  session's own participants are viewable; nothing else under `~/.claude`
  is. Every viewer root now has a rule for what stays hidden below it — the
  repository's `.env*`, `.git/`, key and credentials files are refused while
  `.github/`, `.gitignore` and the like stay viewable.
- **`cl_stale_refs` reports 4 claims on this project's library instead of
  25**: the dated round files count as history, and a symbol whose
  "deleted"/"renamed" sits on the next wrapped line is read as retired.
- The hook body quotes the bot-hq binary path on Unix, so an install under
  a path with a space no longer breaks every hook.

### Deferred

- Enforcing `Content-Type: application/json` on the signaling server (logged
  only this release; the observed client always sends it).
- Gating the `webview_*` tools behind a capability (a parity-oracle decision
  for the user; with fail-closed auth they are reachable only by registered
  agents).
- `react-router-dom` 7 (an open-redirect advisory with no reachable sink in
  a Tauri webview; the fix is a major bump) and the vite 7 / vitest 4 dev
  tooling majors.
- The history rewrite plus the GitHub Support ticket for the six immutable
  `refs/pull/*/head` refs — required before the repository goes public
  again; "scrubbed" is not "clean".
- The in-repo Homebrew cask (`packaging/homebrew/bot-hq.rb`) stays at 1.0.4:
  the tap cannot serve a private repository's release assets, so there is
  no DMG checksum to record. Bump it and copy it to the tap when the
  repository is public again.
- macOS signing + notarization, the AppImage/RPM work, Windows child-process
  reaping and the ConPTY EOF path — unchanged from 1.0.1's list.

## [1.0.4] — 2026-09-01

The work-scope release: sessions kept losing the thread between days — each
one re-interpreting what the project was actually working on, with the
knowledge living in voice calls, chat scroll, and post-hoc reconstructions.
This release gives every project a durable, user-readable answer. It is also
the first tagged release since 1.0.2, so installed apps pick up 1.0.3's
review-layer work with it.

### Added

- **Per-project `focus.md` — the work-scope knowledge base.** A project CL
  may now carry a `focus.md` holding the work scopes currently in flight —
  one section per open scope (a project can run several at once): what is
  actually being worked on plus the absolute truths established about it,
  each with its provenance (what was measured, when, and as which identity),
  written to be read by the user directly, not as agent shorthand. When it
  exists, its whole body rides every participant's system prompt at
  spawn — reviewer included, since scope-watch is now a named review
  dimension; when it doesn't, the prompt carries a one-line creation trigger
  instead. The universal rules gained the full discipline: open a scope's
  section at the Plan boundary (defaulting to opening one when a scope's
  length is unknown), append truths as you learn — each entry naming its
  scope, corrections opening with line-start `SUPERSEDES:` markers —
  reorganize only at a Plan boundary, and clear PER SCOPE once that scope
  settles (graduate its residue to the project CL, remove its section,
  `confirm_shrink: true`; the file survives for the scopes still open) —
  never at close-out, where writers are context-poorest. Oversized bodies
  render head + tail around a loud truncation marker (the tail carries the
  newest truths), and a size or supersession-density advisory schedules the
  reorganize. Distilled from the 2026-08-31 client-project session dissection,
  where the scope's knowledge lived in a voice call and a post-hoc
  reconstruction.

## [1.0.3] — 2026-08-27

The review-layer release: a full-day dissection of why agent errors were
reaching the user found the turn ring silently starving the reviewer in 49%
of two-participant sessions (137 flagged gaps, worst 478 minutes, always the
reviewer's side), a review channel with no reverse direction, and advisory
findings dying undispositioned at close (92%). Everything here follows from
that diagnosis, and the fixes were field-verified live before release: the
same session shape that starved its reviewer for 97 minutes in the morning
dealt it within one second, all afternoon.

### Fixed

- **Tray and gate answers no longer reset the turn rotation to the front.**
  Every approval used to re-deal the executor, so an executor chaining gate
  parks starved the reviewer of turns entirely — both errors that shipped to
  GitHub from the measured session went out inside such a window. A typed
  message still resets (the user steering); an answer releases the ring and
  steps onward from the anchor, so the participant after the asker is served.
- **Anti-starvation backstop:** on every user-row deal, any active
  participant sitting on 10+ undelivered peer texts is served one pre-empting
  turn through the summons queue — covers the typed-message-chain shape the
  root fix cannot.
- **Phase-doc routing keys on reviewer shape** (`file_finding` without
  `edit_files`), so granting an executor `file_finding` — the reverse review
  channel — no longer reroutes its I/P/A/V docs into the reviewer co-doc slot.
- **Chat file links no longer point at paths quoted inside command text.** A
  path inside a quoted query, grep pattern, or pasted output became a
  clickable candidate the viewer then refused; bare-path extraction now
  qualifies whole shell words only.

### Added

- **Outward-publish review precondition:** a gated `gh`/`curl` command may
  park for approval only after the session's reviewer has been *delivered*
  its content (full-body match, file or inline; content-free mutations check
  the turn timeline instead). The refusal teaches the two-turn ritual; a
  solo roster skips the check loudly; a downed reviewer is escaped by the
  existing user-approved override; an unchanged body re-parks after a reject
  without re-review.
- **Starvation visibility:** the session roster shows an amber
  `N unread` chip when a participant crosses the summons threshold — computed
  backend-side from the scheduler's own constant, so the UI and the ring
  cannot disagree. 137 flagged gaps went undiagnosed because a starved
  reviewer was indistinguishable from a quiet one.
- **Open advisory findings surface at session close:** `close_session`
  lists them once before proceeding (never blocking), and the findings
  banner counts them while the session runs. Field-verified at first
  contact: all 7 of the measured session's advisories were dispositioned in
  the final minute instead of archiving silently.
- **"Set all" on Gated Commands** (global Settings and the session gear):
  one control flips every keyword row between Gate and Auto-allow.
- **HANDS role guidance for the reverse review channel** (suggested-pair
  prose): when the role holds `file_finding`, prefer advisory severity for
  reviewer-directed findings — a blocking finding the executor files gates
  its own commits.

## [1.0.2] — 2026-08-26

Hotfix for the Windows upgrade path. The items 1.0.1 deferred "to 1.0.2"
(PTY death, DSR responder, the Windows CI job going required, AppImage
de-bundling, Job Object reaping, macOS signing) are untouched here and carry
to the next release.

### Fixed

- **Windows: upgrading from any 1.0.0-rc / 1.0.0 install no longer exits at
  launch** with "migration 1 was previously applied but has been modified".
  Those builds came off a CRLF checkout and stamped CRLF migration checksums;
  1.0.1 embedded the same migrations with LF endings and refused every
  upgraded database within a second, before any window (fresh installs and
  macOS/Linux were never affected — no migration had changed). Migration
  checksums are now line-ending independent, and a database stamped by a
  CRLF build is repaired in place on first open (one INFO log line names the
  rewrite). One-way: once this build has opened a database, 1.0.0 and
  earlier Windows builds refuse it with the same message — do not roll back
  past this release after upgrading.
- **Windows: untouched HANDS/EYES no longer show "Differs from the shipped
  default"** on installs upgraded from a 1.0.0-rc / 1.0.0 build. Those builds
  stored the role prose with Windows line endings, so the Roles tab compared
  it against the shipped default and found every line changed — offering a
  diff of the whole prose and a Reset that rewrote all of it — when nothing
  but the line endings differed. A one-shot migration normalises the stored
  prose (the roles' edit timestamps do not move), and the comparison and the
  diff now ignore line endings.

### Added

- **A native error dialog when bot-hq fails before its window opens** — a
  data-dir problem, a failed migration, "bot-hq is already running" from a
  second launch, a port that will not bind. It carries the full error chain
  and names the log directory (saying so when logging had not come up yet);
  the same text goes to the log at ERROR level and, as before, to stderr
  with exit code 1. Skipped on a non-interactive Windows window station
  (service, OpenSSH session, non-interactive scheduled task — a message box
  there would block unseen) and when `BOT_HQ_NO_STARTUP_DIALOG=1` is set
  for headless or scripted launches.

## [1.0.1] — 2026-08-26

### Fixed

- **Windows: approved gated commands actually run.** The Tool Gate resolved no
  shell in a GUI process (`sh` is not on PATH), so every *approved* command
  failed with "program not found" — confirmed in the shipped 1.0.0 build. The
  gate now resolves its shell from Git-for-Windows.
- **Windows: agents spawn with the user's MCP servers.** `HOME` is unset on
  Windows, so user-level config paths resolved to nothing and agents silently
  got zero user MCP servers; panic telemetry also hashed paths unredacted. A
  portable home resolver (with a standing guard test) replaces every direct
  `HOME` read.
- **Windows: `terminal_exec` submits with a carriage return** (consoles ignore
  a bare LF). Known limit: if the Terminal tab was never opened, the ConPTY
  startup query has no responder and the tool can still stall — deferred with
  the PTY work below.
- **Windows: Context Library keys are forward-slashed everywhere**, with a
  one-shot in-place migration for databases written before the fix.
- **Linux: the AppImage opens a window on modern Mesa** via
  `scripts/install-appimage-linux.sh`, which strips the payload's stale
  Wayland client libraries (the underlying packaging defect — the AppImage
  still bundles them — is deferred; see below). The silent no-window failure
  is documented in `docs/FEDORA-LINUX-COMPAT.md`.
- **Linux: spawned processes no longer inherit the AppImage's environment.**
  The launcher's `LD_LIBRARY_PATH`/`PYTHONHOME`/etc reached every child, which
  broke host `git` over HTTPS, `curl`, `python3` and made `gsettings` return
  wrong answers inside sessions. A structural scrub (`src/appimage_env.rs`)
  drops payload-rooted entries at all four spawn sites; source builds are
  untouched.
- **Linux: native controls (dropdowns, scrollbars) render dark on dark
  desktops** — `:root { color-scheme: dark }`, measured against a forced
  light GTK theme with two independent probes.
- **`webview_screenshot` works off macOS.** It hardcoded
  `/usr/sbin/screencapture` on every platform (and its error text pointed at
  a macOS settings pane). Now platform-gated: macOS `screencapture`; Linux
  tries Spectacle → grim → ImageMagick → gnome-screenshot; Windows returns an
  explicit unsupported error.
- **Chat-stream file viewer opens `~/...` paths and Context Library files.**
  Tilde paths were treated as repo-relative (ENOENT with a misleading
  message), and the library was outside the viewer's allowed roots. Dotted
  entries under the library (`library/.git/**`) stay unreadable. Limit: a
  project whose CL lives at a custom `cl_path` outside `~/.bot-hq/library` is
  still outside the viewer's scope.
- **Windows notifications: the app can now tell you they're off.** The
  Settings test button reads the `ToastEnabled` master switch (the one signal
  that carries information — the plugin's permission API is a compile-time
  constant on desktop) and warns when Windows has toasts disabled OS-wide.

### Added

- **Starter safety defaults, offered once** (mirrors the roles offer): a
  basic Tool Gate keyword list (destructive commands only — `rm -r`, `sudo `,
  disk writers, `git reset --hard`, `git clean -f`) and a basic general
  policy (`push_gate: ask`, `force_push: blocked`, empty commit word-list
  with commented examples). Offered on fresh installs and on upgrades that
  never wrote the config file; an existing config suppresses its offer and is
  never overwritten (asserted byte-identical). Cards on Settings → Tool Gate
  and Policy; a dismissible Dashboard banner deep-links to them. Note: the
  escalation keyword also catches `sudo dnf` / `sudo apt` — edit the list to
  taste; and a project policy cannot relax `push_gate` back to `auto` (use
  the session's gear toggle).
- **New Context Library projects seed three structured starters** —
  `conventions.md`, `notes.md`, and (new) `decisions.md`, with section
  headings instead of one-line stubs.
- **Reopening a session says when no learnings delta was recorded** for its
  project, softly (an interrupted close and a deliberate write-nothing are
  indistinguishable on disk). In practice the note is rare on actively-worked
  projects: any CL write for the project since the session began counts.
- **Fail-loud webview startup watchdog**: if no page finishes loading within
  30s the app says so on stderr, in the log, and as a `webview_launch_failed`
  diagnostics event — the 1.0.0 Fedora failure produced no text at all.
  Startup only: a webview that loads and later dies is not covered.
- **Role prose view-diff and reset-to-default** against the shipped example
  pair (the 1.0.0 release-notes promise). Reset fills the editor; you still
  save.
- **CI test jobs** (`.github/workflows/test.yml`): `cargo test` + the
  frontend suite on ubuntu-22.04 and macOS as required jobs — no CI ran any
  tests through 1.0.0. The Windows job runs the full fail-closed suite but is
  advisory (`continue-on-error`) until the five known ConPTY failures are
  fixed or named: **the first Windows run of this workflow names the failing
  tests — annotate them by measured name, then flip the job to required**
  (tracked under Deferred below).

### Changed

- `PLAN.md` and `PROGRESS.md` are retired — 1.0.0 closed the build-out. This
  changelog carries what changes per release; `ARCHITECTURE.md`/`CODEBASE.md`
  say what bot-hq is and where things live; git history carries the rest.
- Telemetry/watchdog call-site guard tests scan only uncommented lines — a
  `//` comment-out now fails them. Narrower hole, not closed: a `/* */` block
  would still pass.

### Deferred to 1.0.2

- **Windows Terminal-tab PTY death** (an exited shell looks alive; ConPTY
  delivers no EOF) and the **DSR responder** for `terminal_exec` without a
  mounted Terminal tab — needs live Windows mileage.
- **Windows CI job → required**, once the first run on main names the five
  failing tests and they are annotated by measured name.
- **AppImage de-bundling** (stop shipping `libwayland-*` + `libepoxy`) and/or
  an **RPM** — the bundled-WebKit-vs-host-libstdc++ crash cannot be fixed
  from this repo and is the third independent argument for a distribution
  package.
- **Windows child-process reaping** via a Job Object (new unsafe FFI — lands
  only behind the Windows CI job).
- **macOS signing + notarization** — consciously deferred by the user at
  1.0.0 and again here (Gatekeeper banner-only by choice). The how is in
  `docs/SIGNING.md` (cert → repo secrets → uncomment the workflow's `APPLE_*`
  env); doing it removes the right-click-Open friction and unblocks a future
  auto-updater.

## [1.0.0] — 2026-08-25

First public release: the agent harness — sessions with user-defined roles,
IPAV phase discipline with per-phase documents, the two-layer policy
enforcement (MCP tools + git hooks), Tool Gate approvals, Context Library
with indexed retrieval, session terminal, plugin runtime, opt-in diagnostics,
and the packaged installers (macOS universal `.dmg`, Windows NSIS `.exe`,
Linux `.deb`/AppImage) with the Homebrew tap.
