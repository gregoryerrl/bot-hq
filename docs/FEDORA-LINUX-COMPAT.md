# Fedora / modern-Mesa AppImage incompatibility

Status: **root-caused and fixed** via `scripts/install-appimage-linux.sh`.
Underlying packaging defect in the release AppImage is **still open**.

## Symptom

On Fedora 44 the `bot-hq_1.0.0_amd64.AppImage` appears to do nothing. No
window is ever mapped, no dialog, no crash, and the process stays alive
indefinitely. Running it from a terminal prints the normal startup log and
then stops at:

```
INFO bot_hq: Tauri setup complete; webview launching
```

Because the process does not exit, the failure reads as a hang rather than
an error.

## What is actually happening

The Rust backend starts correctly and completely. Every startup side effect
lands on disk:

- `~/.bot-hq/.local/bot-hq.db` is created and written
- `.local/lock` and `.local/signaling-addr` are written
- the signaling server and the LLM normalizing proxy both bind ports
- `~/.bot-hq/.local/logs/bot-hq.<date>.log` records a clean startup

Only the webview fails. With stderr captured, the last line is:

```
Could not create default EGL display: EGL_BAD_PARAMETER. Aborting...
```

Inspecting the process tree makes the split obvious. `WebKitNetworkProcess`
spawns and survives; `WebKitWebProcess` — the process that owns rendering —
aborts during EGL initialization and never comes back. With no web process
there is no surface to map, so no window appears and nothing is drawn.

## Root cause

The AppImage is built on Ubuntu 22.04 and bundles that era's graphics
client libraries:

```
libwayland-client.so.0
libwayland-egl.so.1
libwayland-server.so.0
libwayland-cursor.so.0
libepoxy.so.0
```

It does **not** bundle `libEGL` or `libGL`. Those are deliberately left to
the host, which is correct — GPU drivers must match the running kernel and
cannot be vendored.

That split is the bug. At runtime the AppImage's `LD_LIBRARY_PATH` puts the
old bundled Wayland client libraries ahead of the host's, while `libEGL`
still resolves to the host copy. On a distribution whose Mesa is close to
the AppImage's build base the two happen to agree. On Fedora 44 they do
not:

| component        | version on this host |
| ---------------- | -------------------- |
| Mesa             | 26.1.7               |
| bundled wayland  | Ubuntu 22.04 era     |
| GPU / driver     | Intel UHD (CML GT2), i915 |
| session          | KDE Plasma, Wayland  |

Host Mesa's Wayland EGL platform calls into the bundled, much older
`libwayland-client`, the handshake fails, and EGL rejects the display with
`EGL_BAD_PARAMETER`.

This is a library-set mismatch, not a renderer problem. That distinction
matters because it rules out the usual workarounds — see below.

## Workarounds that do NOT work

All of these were tested on the affected host and all still abort with
`EGL_BAD_PARAMETER`:

- `WEBKIT_DISABLE_DMABUF_RENDERER=1`
- `WEBKIT_DISABLE_COMPOSITING_MODE=1`
- `GDK_BACKEND=x11` (forcing XWayland)
- `GDK_BACKEND=x11` combined with both WebKit flags above

Forcing X11 does let `WebKitWebProcess` spawn, which looks like progress,
but EGL still fails and no window is ever mapped. Anyone debugging this by
process count alone will misread that state as a partial success.

## The fix

Delete the five conflicting libraries from the payload so the host's own
matching copies load. Everything else in the payload — WebKit itself, GTK,
the app binary — is kept as shipped.

`scripts/install-appimage-linux.sh` does this and installs the result as a
normal desktop application:

```sh
scripts/install-appimage-linux.sh ~/Downloads/bot-hq_1.0.0_amd64.AppImage
```

It extracts the AppImage, removes the five libraries, installs the payload
to `~/.local/lib/bot-hq`, and writes a launcher at `~/.local/bin/bot-hq`
plus a `.desktop` entry so the app appears in the application menu.
`PREFIX` overrides the install prefix.

## Verification

Performed on the affected host, against the installed launcher rather than
a temporary copy:

- `WebKitWebProcess` spawns and stays alive
- zero `EGL_BAD_PARAMETER` occurrences in captured stderr
- the window maps and the UI renders — Dashboard, Context Library, Plugins
  and Settings tabs, and the first-run welcome checklist, confirmed by
  screenshot
- relaunching through `~/.local/bin/bot-hq` reproduces the working state

Not verified: any distribution other than Fedora 44, any non-Intel GPU, and
X11-only (non-Wayland) sessions. The mechanism predicts the same failure
and the same fix wherever host Mesa is substantially newer than Ubuntu
22.04, but that is reasoning, not a measurement.

## Recommended follow-up (release pipeline)

The install script repairs an already-built artifact on the user's machine.
It does not fix the artifact. Options, roughly in order of preference:

1. **Stop bundling the Wayland client libraries and `libepoxy`.** They are
   part of the same contract as `libEGL` — they must track the host. This
   is the direct fix and is what the script does by hand.
2. **Ship the Fedora-relevant path as an RPM**, letting the distribution
   resolve the graphics stack normally.
3. **Document the failure in `INSTALL.md`** so the symptom is searchable.
   The silent no-window hang gives a user nothing to search for, which is
   what made this expensive to diagnose.

Item 3 is done on this branch. Items 1 and 2 are packaging changes and are
left open deliberately — the AppImage build is not reproducible from this
host, so a change there could not have been tested here.

---

## Sign-off

Investigated and written by **Claude Opus 5** (claude-code), acting for
Gregory (gregoryerrl@gmail.com).

Investigation date: 2026-08-25
Host: Fedora 44, kernel 7.0.10-201.fc44.x86_64, KDE Plasma on Wayland
Artifact examined: `bot-hq_1.0.0_amd64.AppImage` (88111608 bytes)

Every claim in the Symptom, What-is-actually-happening, Root-cause,
Workarounds and Verification sections was reproduced on that host during
this investigation. Claims about other distributions, other GPUs and X11
sessions are explicitly marked as untested inference above and were not
measured.

No cryptographic signature is attached: this machine has no GPG key and no
SSH signing key configured, and none was generated for this purpose. The
commit carries a DCO `Signed-off-by` trailer instead. If a verifiable
signature is wanted, configure a signing key and re-sign the commit.
