# bot-hq — User Manual

How to configure and drive bot-hq properly, end to end. This is the
user-facing companion to [`ARCHITECTURE.md`](../ARCHITECTURE.md) (how it
works inside) and [`INSTALL.md`](../INSTALL.md) (getting it onto your
machine).

bot-hq is a desktop **agent harness**: you configure roles, start a
*session* on one of your repos, and role-playing AI participants (each a
`claude-code` subprocess) do the work — investigating, planning, editing,
verifying — while policy gates keep commits, pushes and dangerous commands
under your control. You stay the conductor: everything that matters stops
at you.

---

## 1. First-run setup

### Prerequisites

- **claude-code CLI**, installed and authenticated (`claude`, then log in
  and `/exit`). It is the only model connector — every participant runs
  through it, whatever model it's pointed at. On Windows use the *native
  installer* (`irm https://claude.ai/install.ps1 | iex`), not the npm
  package.
- **git** — sessions run against git repositories.
- Your data lives in `~/.bot-hq/` (database, Context Library, logs,
  config). It survives upgrades and uninstalls.

### First launch

1. macOS: unsigned build — right-click → Open (Sequoia: System Settings →
   Privacy & Security → Open Anyway). Windows: SmartScreen → More info →
   Run anyway.
2. A one-time **DIAGNOSTICS** card asks whether to share anonymous crash
   reports and usage counts with the bot-hq author — never code or
   prompts. Both answers are final until you change them in **Settings →
   Diagnostics**; see [`PRIVACY.md`](../PRIVACY.md) for exactly what is
   sent where.
3. A fresh install has one neutral **agent** role (every capability, no
   instruction text) and the Roles tab offers — once — to install the
   example **HANDS**/**EYES** pair (an executor and an adversarial
   reviewer). Take the offer if you want a working two-participant setup
   out of the box.

### The three setup steps (the dashboard walks you through them)

1. **Add a project** — Context Library tab (or pick a repo folder when
   starting a session). A project ties sessions to a repository and gives
   agents a place to keep durable notes (see §6).
2. **Add a model** — Settings → Models (optional: without one,
   participants use claude-code's built-in default). A saved model is a
   name + base URL + token; **Test** checks it's reachable. Non-Anthropic,
   Anthropic-compatible gateways work — the CLI is pointed at them per
   participant.
3. **Create a session** — the **+ New session** button (⌘N).

---

## 2. Roles (Settings → Roles)

A role is a reusable participant template you own:

- **Role instruction** — free-form markdown injected into every session
  the role joins: identity, voice, priorities. It is the identity layer
  only — bot-hq's universal rules and the capability-derived rules are
  composed after it, and text here can never grant a capability the
  checkboxes withhold. Clearing it stores nothing: the role then runs on
  the universal rules and its capabilities alone.
- **Capabilities** — the actual permission set (edit files, run shell,
  ask the user, file review findings, approve findings, write the Context
  Library, close the session, …). The tool gate enforces these
  mechanically; a participant without a capability has the matching tools
  refused.
- **Participation mode** — active (takes turns in rotation) or
  on-mention (sits out until summoned with `@role`).
- **Default model / effort** — what the role runs on unless a session
  overrides it.

Two things worth knowing:

- **Your edits are permanent**: once you change a role's text, bot-hq
  never auto-updates it — including across app upgrades.
- A useful pattern is one **executor** role (edits, runs commands,
  commits) and one **reviewer** role (reads, files findings, approves) —
  the example pair is exactly this, and the reviewer's *blocking findings
  mechanically gate `git commit`* until the executor fixes or rebuts them.

---

## 3. Sessions — the core loop

### Starting one

**+ New session**: pick the project (or a repo folder), optionally check
**worktree isolation** (the session works in its own git worktree instead
of your live checkout — recommended when you keep working in the repo
yourself), then build the roster: one or more participants, each a role
plus optional per-session overrides (display label, model, effort). One
participant = solo executor, no review laps; two or more = the rotation
below.

### How a session runs

- Participants take **turns in a fixed rotation**. One works at a time;
  everything posted since a participant's last turn is delivered when its
  turn starts.
- **Your messages land between turns.** While someone is working you can
  type and send — it stages and delivers at the next turn boundary. The
  **Pause** button is the only true interrupt.
- Substantive work walks four phases — **Investigate → Plan → Apply →
  Verify** — shown as a chip on the session. Each phase leaves a document
  in the matching **I/P/A/V tab**: the investigation record, the plan, the
  deliverable (the **A** tab also renders the session's live `git diff`,
  color-coded), and the verification evidence. Participants vote to
  advance; the chip moves on consensus.
- The **Workspace / Context / Terminal** subtabs show the working tree,
  what the agents retrieved from your Context Library, and a real
  terminal in the session's repo that agents can type into — visible to
  you live.

### When it needs you (the stopping surfaces)

- **The bell (tray)** counts parked **questions**. Agents ask via
  structured choices; your picks deliver at the next turn boundary. A
  question stops nothing — the session keeps working while it waits.
- **Approvals and gated commands** stop the session until you decide:
  when an agent's command matches a Tool Gate keyword (§5) it parks with
  the exact command shown — **Approve** runs it, **Reject** blocks it.
  Pushes under the `ask` policy park the same way, per push.
- **Halts**: when the next move is genuinely yours, the session stops and
  the banner tells you why. A **TEMPORARY HALT** shows a countdown — the
  session is waiting on something external (CI, a build) and wakes itself.
- **Findings**: a reviewer's blocking finding shows on the session and
  gates commits until resolved (fixed or rebutted — rebuttals are shown
  to you).
- **OS notifications** (§5, Notifications): questions, approvals, gates
  and halts escalate to your machine's notifications while the window is
  unfocused, so you can walk away.

### Ending one

Sessions ask before closing. Closed sessions keep their documents and
history (Settings → Archive).

---

## 4. The safety model (what agents can and cannot do)

- **Capabilities** gate every signaling tool per participant (§2).
- **The Tool Gate** (Settings → Tool Gate) is a global keyword list over
  agent shell commands: `gate` keywords park the command for your
  approval; `auto_allow` keywords skip prompting. Keywords are
  case-insensitive substrings of the whole command — prefer multi-word
  forms (`gh pr`, `rm -rf`). Live sessions use the snapshot taken at
  spawn; edits apply to new sessions.
- **Git hooks**, installed per working repo, enforce policy even if an
  agent never reads the chat: the commit-message hook blocks forbidden
  words (Settings → Policy / per-project `policy.yaml`), the findings
  gate blocks commits under unresolved blocking findings, and the
  pre-push hook implements the push policy. Hook refusals are logged to
  the **Violations** tab.
- **Session Settings (the gear on a session)** carry the per-session
  policy toggles, inherited from global → project → session: `push_gate`
  (`auto` = pushes go through; `ask` = every push parks an
  Approve/Reject), `force_push` (blocked/allowed), and the action-gate
  behavior. Destructive operations (force-push, hard resets, branch
  deletion) additionally require your explicit per-action say-so.
- Agents treat production databases as read-only, and outward actions
  (publishing, posting, sending) require your explicit in-session
  instruction — gates surface them when commands are involved.

---

## 5. Settings reference

| Tab | What it configures |
| --- | --- |
| **Roles** | Your participant templates: instruction text, capabilities, participation mode, default model/effort (§2). Landing tab. |
| **Models** | Saved endpoints (name, base URL, token) + reachability **Test**. |
| **Claude Config** | The claude-code configuration lens: what the spawned CLI inherits, with per-role overrides. |
| **Tool Gate** | The global `gate` / `auto_allow` keyword lists (§4). |
| **Policy** | The general policy (commit-message forbidden words, push defaults) that projects and sessions inherit. |
| **Violations** | The audit log of policy denials and gate refusals (`violations.jsonl`). |
| **Feedback** | Harness-friction items agents filed for a future maintenance session. |
| **Promptcodes** | Reusable prompt snippets. |
| **Archive** | Closed sessions and their documents. |
| **Updates** | Installed vs latest release, with **Check now**. A banner also appears app-wide when a newer release exists — **Download** opens the release page; installs are manual by design. |
| **Notifications** | The OS-escalation toggle (§3) + **Send test notification**. Repeats are cooldown-suppressed; simultaneous waits coalesce into one summary. Linux needs a notification daemon; Windows toasts assume the installed app. |
| **Diagnostics** | The opt-in telemetry toggle, your install id (minted on enable, deleted on disable), locally-queued bytes, and the endpoint override for self-hosting the sink ([`packaging/telemetry-worker/`](../packaging/telemetry-worker/)). |

---

## 6. The Context Library

`~/.bot-hq/library/` — the durable, human-editable memory agents read
FIRST on every substantive task. Per project it holds conventional files
(`conventions.md` — how your repo works: formatter, test commands, commit
rules; `notes.md` — gotchas; `decisions.md` — an append-only decision
log; `policy.yaml` — machine-enforced gates), plus whatever else you or
your agents keep there. Cross-project files live at the root, including
`custom-instructions.md` (prose added to every agent) and
`custom-general-rules.md` (your additions to the universal rules).

The **Context Library tab** is a two-pane editor over all of it: browse,
edit, create, and describe folders. Agents with the library-write
capability append session learnings at close; you prune. The tab also
registers projects (point it at a repo) and carries the retrieval
**Measurement** view.

Treat it as study notes, not a textbook: what the code can't tell an
agent — conventions, wiring, why-it's-weird-here. It is the highest-lever
place to invest five minutes before setting agents loose on a repo.

---

## 7. Plugins

The **Plugins tab** installs static frontend bundles that run sandboxed
(iframe, no direct system access — everything they do goes through one
audited proxy with per-plugin grants you consent to at install). Install
from a URL or a local directory; enable/disable per plugin. A plugin
contributing a panel gets its own top-bar tab.

---

## 8. Troubleshooting

- **Blank window on launch (Windows)** — install the WebView2 Evergreen
  runtime.
- **"bot-hq is already running"** — the single-instance lock; quit the
  other copy (a stale lock from a crash is taken over automatically).
- **Agents won't spawn / model Test fails** — verify `claude` runs and is
  authenticated in a terminal; on Windows confirm `where.exe claude` ends
  in `claude.exe` (native installer, not the npm shim).
- **No update banner but you expect one** — the check runs at launch and
  only sees *published, non-prerelease* releases.
- **No OS notifications on Linux** — install/run a notification daemon
  (mainstream desktops ship one); **Send test notification** tells you if
  sends fail.
- **Commit blocked** — read the hook's message: a forbidden word
  (reword it) or an unresolved blocking finding (resolve it in-session).
  Don't bypass hooks; that's the safety model working.
- **Telemetry shows queued bytes but nothing sends** — you're offline or
  the endpoint is unreachable; batches retry (launch + every 30 min) and
  the queue is capped at 1 MB, oldest dropped first.
