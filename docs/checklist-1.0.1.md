# 1.0.1 — what shipped, and how to check each one

For Gregory and the windows / fedora re-run sessions (this file is the
promoted session checklist from the macOS release session, 2026-08-26).

Release: <https://github.com/gregoryerrl/bot-hq/releases/tag/v1.0.1> ·
31 first-parent commits over 1.0.0 (58 total counting the merged branch work)

## Ship state (verified at release time — spot-check freely)
- [x] `curl -s https://api.github.com/repos/gregoryerrl/bot-hq/releases/latest` → `v1.0.1` (independently re-checked: draft=false, exactly 4 artifacts)
- [x] 4 artifacts on the release: universal `.dmg` / `x64-setup.exe` / `amd64.deb` / `amd64.AppImage`
- [x] Homebrew: cask + tap both at 1.0.1, sha `35b474af…` — `brew upgrade --cask gregoryerrl/bot-hq/bot-hq` on any brew install
- [x] CI: first-ever test workflow — ubuntu + macos green (required); windows advisory-red naming its 7 failures (by design, flips to required in 1.0.2)
- [x] Both platform branches deleted (work is in the merge history)
- [x] PLAN.md / PROGRESS.md retired → `CHANGELOG.md` is the record

## macOS (relaunch into 1.0.1 first — brew upgrade or the dmg)
- [ ] **Update banner fires on the old build BEFORE upgrading** — direct observation, not the curl inference.
- [ ] **Chat file links work**: click any `~/.bot-hq/library/...` path in an old chat → file opens (the reported bug). `library/.git/config` must refuse.
- [ ] **No offer cards on this machine** — correct, not missing: config files exist, so the offers are suppressed by design.
- [ ] **Roles → HANDS or EYES**: "Differs from the shipped default" hint + View diff + Reset to default under the instruction box (release-notes promise).
- [ ] **New CL project** (any name): born with `conventions.md` + `notes.md` + `decisions.md`, all with section headings.
- [ ] **Reopen any closed session**: the REOPENED row may carry the soft "no learnings delta was recorded" note (rare on an actively-worked project).

## Windows re-run (installer: `bot-hq_1.0.1_x64-setup.exe`)
- [ ] **Update banner on the installed 1.0.0 before upgrading** (second direct observation).
- [ ] App boots; **Dashboard shows the starter-offers banner** (sessions exist, no config files → backfill arms both offers).
- [ ] **Accept the GATES offer and verify the list actually landed** — Settings → Tool Gate must show the 7 basic keywords. ⚠️ This is the FIRST LIVE test of the `120806f3` fix anywhere: on the macOS throwaway instance the pre-fix build was clicked — the policy starter installed but the gate Install was a silent no-op (that click became the finding). The published build carries the fix; this click proves it.
- [ ] Approve-and-run a gated command — **it actually executes** (the F6 shell fix; 1.0.0 silently failed every approved command).
- [ ] **Click a `~/...` file path in chat** — the viewer's home expansion resolves `USERPROFILE` on Windows via `paths::home_dir()`, an arm that has never run on a real Windows box.
- [ ] Agents spawn WITH the user MCP servers (the HOME fix — 1.0.0 spawned them with none).
- [ ] Terminal tab: `terminal_exec` works with the tab open (CR fix). Known 1.0.2 gap: exited shells still look alive; the tool can stall if the tab was never opened.
- [ ] Toasts: Settings → Notifications warns if the OS master switch is off (new); test button delivers when it's on.
- [ ] CL paths render forward-slashed; a pre-1.0.1 DB migrates its `\` keys in place on first boot.

## Fedora re-run (release AppImage + `scripts/install-appimage-linux.sh` — de-bundle is 1.0.2, the script is still the path)
- [ ] Script installs; window opens (EGL repair).
- [ ] **Dashboard shows the starter-offers banner**; accepting the gates offer must land the 7 keywords (same `120806f3` proof as Windows).
- [ ] Inside a session: `git ls-remote` over HTTPS / `curl` / `python3` all work in the agent's shell (the env-scrub — all dead in 1.0.0).
- [ ] Dropdowns/scrollbars render dark (color-scheme fix — confirmed on the from-source build; this checks the packaged one).
- [ ] `webview_screenshot` captures via Spectacle (first packaged-build check of the Linux chain).
- [ ] If a window ever fails to appear: stderr + log now say so within 30s (`webview_launch_failed` also lands in diagnostics) — the 1.0.0 silent hang is gone.
- [ ] Diagnostics on → D1 gains the launch row (query in `packaging/telemetry-worker/README.md`).

## Known-deferred (CHANGELOG "Deferred to 1.0.2" — nothing below is a regression)
Windows PTY-death + DSR responder · Windows CI → required (7 named tests) ·
AppImage de-bundling / RPM · Job Object reaping · macOS signing + notarization
