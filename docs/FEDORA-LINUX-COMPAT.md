# Fedora / modern-Mesa AppImage incompatibility

Status: **root-caused and fixed** via `scripts/install-appimage-linux.sh`.
Underlying packaging defect in the release AppImage is **still open**.

A second investigation on 2026-08-25, from inside a session running on the
repaired payload, found three further Linux defects — an environment leak into
every spawned child, a light-themed native control set, and a screenshot tool
that never worked off macOS. Those are §§"The environment leak" onward; the
original no-window investigation is unchanged below.

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

## The environment leak (found 2026-08-25, fixed in `src/appimage_env.rs`)

### Symptom

Inside a bot-hq session running from the repaired payload, host tools fail in
the agent's shell and in the app's own Terminal tab:

```
git ls-remote --heads origin
  git-remote-https: symbol lookup error: /lib64/libcurl.so.4: undefined symbol:
      nghttp2_option_set_no_rfc9113_leading_and_trailing_ws_validation
  fatal: remote helper 'https' aborted session          → fetch/pull/push all dead

curl --version   → the identical symbol lookup error
python3 -c ...   → Fatal Python error: Failed to import encodings module
flatpak          → libaccountsservice.so.0: undefined symbol: g_once_init_leave_pointer
spectacle        → liblzma.so.5: version `XZ_5.4' not found (required by libKF6Archive.so.6)
                   libgnutls.so.30: version `GNUTLS_3_7_7' not found
git --version    → works, but warns: libpcre2-8.so.0: no version information available
```

### What is actually happening

An AppImage launches through `AppRun`, which sources
`apprun-hooks/linuxdeploy-plugin-gtk.sh` (the `GTK_*`, `GIO_*`, `GSETTINGS_*`,
`XDG_DATA_DIRS`, `GDK_BACKEND` and `APPDIR` exports) and then execs
`AppRun.wrapped`, linuxdeploy's C shim, which exports `LD_LIBRARY_PATH`, `PATH`,
`PYTHONHOME`, `PYTHONPATH`, `PERLLIB` and `QT_PLUGIN_PATH`. All of it points
into the payload, and **every process bot-hq spawns inherits it wholesale** —
agent subprocesses, the PTY behind the Terminal tab, approved gated commands,
the library's `git push`, and the four installed git hooks transitively.

A child is usually a HOST binary, so it loads the payload's Ubuntu-22.04-era
libraries instead of its own. `ldd /lib64/libcurl.so.4` resolves **twelve**
dependencies into the payload — nghttp2, idn2, psl, the whole krb5 stack,
brotli, unistring, udev, keyutils — which is the entire HTTP/TLS/Kerberos chain
of any host binary that links it. Plain `git` only pulls `libpcre2`, which is
why it merely warns while `git-remote-https` (a separate binary that links
libcurl) dies outright.

`PYTHONHOME` is a different mechanism worth naming separately: `PYTHONPATH` is
additive and harmless, but `PYTHONHOME` *overrides* the interpreter prefix, and
the payload has no python lib directory at all.

### The corollary that costs debugging time

`gsettings` does not fail under the leak — it **lies**:

| | `gtk-theme` | `color-scheme` |
| --- | --- | --- |
| inside an agent shell (leaked) | `'Adwaita'` | `'default'` |
| with the payload vars stripped | `'Breeze'` | `'prefer-dark'` |

The obvious explanation — that the payload's `GSETTINGS_SCHEMA_DIR` shadows the
host schemas — is **wrong**, and was believed here long enough to nearly produce
a false correction against a correct diagnosis. Measured, one variable at a
time:

```
leaked baseline           → 'default'        the lie
-u GSETTINGS_SCHEMA_DIR   → 'default'        still lying
-u GIO_EXTRA_MODULES      → 'default'        still lying
-u LD_LIBRARY_PATH        → 'prefer-dark'    truthful
```

The real mechanism is `LD_LIBRARY_PATH`: host `gsettings` loads the PAYLOAD's
glib/gio, which searches its own compiled-in module path, and that directory
ships exactly one module —

```
payload: libgiognutls.so
host:    giomodule.cache libdconfsettings.so libgiognomeproxy.so
         libgiognutls.so libgiolibproxy.so
```

— so there is **no dconf backend**, and GSettings falls back to returning each
key's schema default. That is why both keys read as defaults rather than as the
payload's own GNOME values.

Any theme or desktop-integration measurement taken inside a session must be
re-taken through a scrubbed environment before it is believed.

### The fix

`src/appimage_env.rs` strips payload-rooted entries from the environment of
processes bot-hq **spawns**, and only there. The rule is structural rather than
a list of variable names, because `AppRun` is generated and can add variables at
any release:

> For every inherited variable, treat the value as a `:`-separated list, drop
> every entry that resolves under `$APPDIR`, and remove the variable entirely if
> nothing survives.

with two exceptions learned in review: `PATH` is never removed (it falls back to
`/usr/bin:/bin` — a child with no `PATH` cannot resolve a bare command), and if
nothing was dropped, **no** operation is emitted at all, so values the rule was
never meant to touch are byte-identical rather than round-tripped through
split/join. It is guarded on `APPDIR` being set *and* the running executable
living under it, so a source build is a strict no-op.

Applied at `agents/spawn.rs::build_command`, `core/terminal.rs::spawn`,
`policy/tool_gate.rs::run_in_shell` and `signaling/bridge/cl_push.rs::git`.

### Why not fix it in `AppRun` or the launcher

Because `WebKitWebProcess` genuinely needs those payload libraries. Unsetting
them before the app starts strips them from the webview too and re-creates the
`EGL_BAD_PARAMETER` no-window failure documented above. The app keeps the
AppImage environment; only what it spawns is scrubbed.

### Workaround for a build that predates the fix

```sh
env -u LD_LIBRARY_PATH -u PYTHONHOME <command>
```

Two variables cover every symptom measured on this host — verified together:
`curl 8.18.0`, `git ls-remote` over HTTPS returning real refs, `python3` ok,
`gsettings` → `'prefer-dark'`, `spectacle 6.7.4`, `Flatpak 1.18.1`.

This is the stopgap for a human on a build that predates the fix. It is NOT an
argument to narrow the scrub itself: `PERLLIB`, `QT_PLUGIN_PATH` and the
`GST_*` pair are still payload-rooted and still go, because the structural rule
does not need to know which breakage each one causes.

---

## Native controls render light on a dark desktop (fixed in `frontend/src/index.css`)

`apprun-hooks/linuxdeploy-plugin-gtk.sh` decides the GTK theme like this:

```sh
gsettings get org.gnome.desktop.interface gtk-theme | grep -qi "dark" \
    && GTK_THEME_VARIANT="dark" || GTK_THEME_VARIANT="light"
export GTK_THEME="${APPIMAGE_GTK_THEME:-Adwaita:$GTK_THEME_VARIANT}"
```

On KDE that probe returns `'Breeze'` — no substring "dark" — so the hook exports
`GTK_THEME=Adwaita:light` on a desktop that is dark (`color-scheme` is
`'prefer-dark'`; the Fedora look-and-feel package is `fedoradark`). It reads the
wrong key.

WebKitGTK draws native form controls — `<select>` and its popup, scrollbars,
spinners — from the GTK theme, not from page CSS, so they render light inside
the app. Captured on this host: the session-header phase `<select>` as a light
pill in a dark header.

The fix is `:root { color-scheme: dark; }` in `frontend/src/index.css`, pinned by
`frontend/src/lib/theme.ts` + `theme.test.ts`.

**Measured, not assumed** (2026-08-25, after the build dependencies were
installed). Whether WebKitGTK honours `color-scheme` for a native `<select>` was
an open question through the whole fix — the pin test only proves the
declaration ships. Settled with a minimal WebKitGTK 2.52.5 client (30 lines of
C against `webkit2gtk-4.1`), run twice under an identical, deliberately FORCED
`GTK_THEME=Adwaita:light` — the exact condition the AppImage hook creates — with
the same markup and only the declaration differing:

| page | native `<select>` |
| --- | --- |
| no `color-scheme` | **white pill, dark text** — the reported bug, reproduced |
| `:root { color-scheme: dark }` | **dark charcoal pill, light text** |

So the declaration overrides the GTK theme rather than merely coinciding with a
dark one; forcing the theme light in BOTH runs is what rules that out. The
from-source app was also built and run on this host under the same forced light
theme and renders correctly. It is the portable lever: the
hook is regenerated by linuxdeploy on every build and is not in this repo, so
patching it would fix one machine and ship nothing — and the same latent bug
exists on any light-mode Windows or macOS host, where no GTK hook is involved at
all. If GTK chrome *outside* the webview ever matters, the durable seam is the
hook's `APPIMAGE_GTK_THEME` fallback, settable from the launcher that
`scripts/install-appimage-linux.sh` writes.

---

## `webview_screenshot` never worked off macOS

`src/tauri_cmd/screenshot.rs` hardcoded `/usr/sbin/screencapture` with **no
`cfg` gate**, and `signaling/jsonrpc.rs` calls it unconditionally. So the MCP
tool failed at runtime on Linux (`screencapture spawn`) and on Windows, in a
release shipped to all three platforms — and its error text then told the user
to open *System Settings → Privacy & Security → Screen Recording*, a pane that
does not exist on their OS. Nothing caught it because no CI job runs the tests.

Now platform-gated: macOS keeps `screencapture`; Linux tries Spectacle → `grim`
→ ImageMagick `import` → `gnome-screenshot`, whichever is present, spawned
through the scrub above (Spectacle is a Qt/KF6 app and dies on the leaked
`LD_LIBRARY_PATH` otherwise); Windows returns an explicit unsupported error.
It falls through to the next backend when one fails.

What was actually measured on this host, stated precisely because an earlier
draft of this section had it backwards:

| backend | result |
| --- | --- |
| `spectacle -b -n -a -o <out>` — the shipped argv, and the one selected here | **works**: exit 0, a 395,522-byte 2050x1164 PNG |
| `import -window root -crop … +repage <out>` — the shipped argv | **fails**: `import: missing an image filename`, no file written — with a real filename supplied. Also fails as `-window root <out>`, with `-display :0`, and under `env -i`. **Not name resolution**: the numeric root id from `xdpyinfo` (`0x400`) fails identically. So ROOT capture specifically does not work here, the error text is misleading, and the cause is **not established** — an earlier draft blamed XWayland, which is a mechanism read off an error message that does not mention one |
| `import -window <id> <out>` | works (343,622 bytes) — the window-id form, which the code cannot use because it has no XID |
| `grim`, `gnome-screenshot` | not installed here; unit tests only |

So `import`'s branch is **unreachable on this host at runtime** — Spectacle is
found first — and would not have worked if it were reached. It is kept because
the root grab is correct on a real X11 session, and the fall-through means a
present-but-inapplicable backend no longer ends the attempt.

The 2050x1164 measurement is also where the size-check tolerance comes from:
Spectacle includes decorations and shadow, so a 1920x1006 window arrives 7%
wider and 16% taller.

---

## The Tauri `APPDIR` warning — known, not fixed

Every launch of an extracted payload logs:

```
WARN tauri_utils: `APPDIR` or `APPIMAGE` environment variable found but this
application was not detected as an AppImage; this might be a security issue.
```

Same root as the environment leak: the GTK hook exports `APPDIR`
unconditionally (`# Workaround to run extracted AppImage`) while no `APPIMAGE`
file exists on the install-script path. Cosmetic, and deliberately left alone —
the alternative is fabricating an `APPIMAGE` path, which would be a lie to
Tauri's own detection.

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
