# bot-hq on Windows — Tester Guide (v1.0.0-rc1)

Thanks for helping test bot-hq on Windows! This is a **pre-release** build
(`v1.0.0-rc1`) — it compiles and bundles on Windows, but this is the first
round of real runtime testing on the platform, so expect a rough edge or two.
Your feedback is exactly what we need.

bot-hq drives AI-assisted coding sessions through two agents — **Brian**
(HANDS, executes) and **Rain** (EYES, reviews) — with policy enforcement.

---

## 1. Prerequisites (install these first)

### a) claude-code — use the **native installer** (important)

In **PowerShell**, run:

```powershell
irm https://claude.ai/install.ps1 | iex
```

This installs a native `claude.exe` onto your PATH. **Do this rather than the
npm install** (`npm i -g @anthropic-ai/claude-code`): the npm package installs
`claude.cmd`, which bot-hq cannot launch reliably on Windows. If you already
have the npm version, install the native one over it and restart your terminal.

> If bot-hq can't find a native `claude.exe`, it will tell you exactly this and
> point you back here — it won't fail silently.

Then authenticate once so the CLI works on its own:

```powershell
claude          # follow the login prompt, then type /exit
```

Confirm it's the native build:

```powershell
where.exe claude     # should show a path ending in \claude.exe (e.g. %USERPROFILE%\.local\bin\claude.exe)
```

### b) Git for Windows

Install from <https://git-scm.com/download/win>. bot-hq's policy hooks run
through Git for Windows' bundled shell, so the standard Git-for-Windows install
is what you want (not a minimal/MSYS-less git).

### c) WebView2 runtime

Already present on Windows 11 and Windows 10 21H2+. If bot-hq shows a blank
window on launch, install the Evergreen runtime from Microsoft:
<https://developer.microsoft.com/microsoft-edge/webview2/>.

---

## 2. Install bot-hq

1. Download `bot-hq_1.0.0-rc1_x64-setup.exe` from the
   [GitHub release](https://github.com/gregoryerrl/bot-hq/releases).
2. Run it. The installer is **unsigned**, so Windows SmartScreen will warn
   *"Windows protected your PC."* Click **More info → Run anyway**. (This is
   expected for a test build — code signing comes later.)
3. It installs per-user (no admin prompt) and launches.

---

## 3. Smoke checklist (please run through this)

Tick these off and tell us where it breaks:

- [ ] **App launches** — you see the dashboard (not a blank window). *(WebView2 OK)*
- [ ] **Add a model** — Settings → Models → add your model + token, click **Test**.
      It should say reachable. *(If claude-code isn't installed right, the Test
      button surfaces a clear message — that's the check working.)*
- [ ] **Create a project** — point it at a folder / git repo on your machine.
- [ ] **Create a session** and open it.
- [ ] **Drive one turn** — send Brian a simple task and confirm he responds.
      *(This proves bot-hq can launch the `claude` agent — the #1 thing to verify.)*
- [ ] **Make a commit** inside a session (have Brian commit something, or commit
      in the session's working folder) and confirm it goes through. *(This
      exercises the policy git-hooks on Windows.)*
- [ ] **Second instance is blocked** — launch bot-hq again while it's running;
      it should refuse to start a second copy ("already running").
- [ ] **No errors on first launch** — nothing crashes during startup.

---

## 4. Known rough edges (so you don't report these as surprises)

- **Unsigned installer** → the SmartScreen warning above. Normal for now.
- **claude-code must be the native `.exe`** (see §1a). The npm `claude.cmd` is
  not supported on Windows yet.
- Git hooks and the secret-token file permissions are new on Windows this
  build — if commits behave oddly or you see a startup warning, that's useful to
  report.

---

## 5. Sending feedback

Anything — bugs, confusion, "this felt weird", or feature ideas:

- Open an issue at <https://github.com/gregoryerrl/bot-hq/issues> (a screenshot
  + what you were doing helps a lot), **or**
- Message Gregory directly with the same.

Rough notes are perfectly fine. Thank you! 🙏
