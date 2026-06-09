# Signing & notarization (upgrade path)

bot-hq's release builds are currently **unsigned**. They work for test users
(macOS: right-click → Open — see [INSTALL.md](../INSTALL.md)), but a
Gatekeeper-clean / SmartScreen-clean experience needs platform signing. This is
the checklist to turn it on — no code changes beyond uncommenting the CI `env`
block.

## macOS — Developer ID signing + notarization

Requires an **Apple Developer account** ($99/yr) and a *Developer ID
Application* certificate.

1. Export the certificate as a base64-encoded `.p12` and gather:
   - `APPLE_CERTIFICATE` — base64 of the `.p12`
   - `APPLE_CERTIFICATE_PASSWORD` — the `.p12` password
   - `APPLE_SIGNING_IDENTITY` — e.g. `Developer ID Application: Your Name (TEAMID)`
   - `APPLE_ID` — your Apple ID email
   - `APPLE_PASSWORD` — an app-specific password (NOT your Apple ID password)
   - `APPLE_TEAM_ID` — your 10-character team id
2. Add them as **repository secrets** (Settings → Secrets and variables → Actions).
3. Uncomment the `env:` block on the "Build bundles" step in
   [`.github/workflows/release.yml`](../.github/workflows/release.yml). Tauri
   signs the app and submits it to Apple's notary service during the build.
4. Cut a release as usual. The `.dmg` is now notarized, and the Homebrew cask's
   Gatekeeper caveat can be dropped.

Refs: <https://tauri.app/distribute/sign/macos/>

## Windows — code signing (optional)

Unsigned `.exe` installers trigger SmartScreen "unknown publisher" (users click
*More info → Run anyway*). An OV/EV code-signing certificate removes the
warning. Windows builds are deferred today (un-`cfg`-gated unix-only code; see
[PLAN.md](../PLAN.md)); wire signing in when the Windows target returns to the
release matrix.

## Full auto-update (future)

The real Tauri updater plugin (silent in-app updates) needs the **same macOS
signing** plus an updater keypair — it verifies each download's `.sig` against a
public key baked in at build time. The current check-for-updates banner (polls
GitHub Releases + manual download) needs none of this and ships today; it
graduates to full auto-install once signing is in place.
