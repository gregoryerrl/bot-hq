# Changelog

Releases, newest first — [Keep a Changelog](https://keepachangelog.com/) shape.
Work between releases accumulates under **[Unreleased]**; a release moves that
block under its version heading. Development history before 1.0.0 lives in git
and in `docs/rebuild-archive/`.

## [Unreleased]

## [1.0.8] — 2026-09-26

Stray turns and two dependency advisories (session s-02101415). No
migrations.

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

- **rustls 0.23.45** — RUSTSEC-2026-0285 (TLS 1.3 handshake messages
  accepted across encryption levels). bot-hq is a TLS client only.
- **react-router 7.18.4** — an open redirect through a backslash in
  `<Link>`/`useNavigate`, and constructor injection in `deserializeErrors`.
  The app renders nothing on a server; `npm audit` now reports no runtime
  vulnerabilities.

### Deferred

- **glib 0.18** (RUSTSEC-2024-0429, an unsound `VariantStrIter`) is pinned
  by Tauri's GTK3 stack on Linux; Dependabot's security update fails on
  every run until Tauri moves to gtk-rs 0.20.
- A live routing check of react-router 7 in the running app, after the next
  relaunch.

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
