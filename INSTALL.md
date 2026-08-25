# Installing bot-hq

bot-hq ships for **macOS**, **Linux** and **Windows** (the release workflow
builds a `.dmg`, `.deb`/`.AppImage` and an NSIS `.exe`; on Windows expect a
SmartScreen prompt for an unsigned build — see `docs/WINDOWS-TESTING.md`).
Builds are on the
[Releases page](https://github.com/gregoryerrl/bot-hq/releases).

## Prerequisite (all platforms)

bot-hq drives AI agents by running the **`claude-code` CLI** as a subprocess, so
it must be installed and authenticated first:
<https://docs.claude.com/en/docs/claude-code>. `git` is also required for the
repositories you point bot-hq at.

(A non-Anthropic model runs through the same CLI, with the base URL and token
swapped per agent — so claude-code is required whatever model you point a
participant at. The built-in agent loop that once avoided it was removed in
rc3 D9.)

A released build is otherwise self-contained — you do **not** need Rust or Node
to run it (those are only for building from source; see [README.md](README.md)).

## macOS (universal — Apple Silicon + Intel)

### Homebrew (recommended)

```sh
brew install --cask gregoryerrl/bot-hq/bot-hq
```

This taps `gregoryerrl/homebrew-bot-hq` and installs the latest release.

### Direct download

Download `bot-hq_<version>_universal.dmg` from the Releases page, open it, and
drag **bot-hq** to Applications.

> **Unsigned build:** this release is not yet notarized, so Gatekeeper will say
> bot-hq "cannot be opened" or "is damaged". On macOS 15 (Sequoia) and later,
> launch it once (it gets blocked), then open **System Settings → Privacy &
> Security** and click **Open Anyway**. Alternatively, clear the quarantine
> flag:
>
> ```sh
> xattr -dr com.apple.quarantine "/Applications/bot-hq.app"
> ```
>
> (On macOS 14 and earlier, right-click the app → **Open** once also works;
> Sequoia removed that bypass for unsigned apps.)

## Linux

### AppImage (any distribution)

```sh
chmod +x bot-hq_<version>_amd64.AppImage
./bot-hq_<version>_amd64.AppImage
```

#### If no window opens (Fedora and other modern-Mesa distributions)

On distributions whose Mesa is much newer than the AppImage's Ubuntu 22.04
build base — Fedora 40+, Arch, recent openSUSE — the app can start without
ever showing a window. It does not crash and does not exit. Run it from a
terminal; if you see

```
Could not create default EGL display: EGL_BAD_PARAMETER. Aborting...
```

then the bundled Wayland libraries are colliding with your host graphics
stack and WebKit's rendering process is aborting. Install with:

```sh
scripts/install-appimage-linux.sh bot-hq_<version>_amd64.AppImage
```

That repairs the payload and installs `bot-hq` to `~/.local/bin` with a
desktop entry. See [`docs/FEDORA-LINUX-COMPAT.md`](docs/FEDORA-LINUX-COMPAT.md)
for the full diagnosis. Setting `WEBKIT_DISABLE_DMABUF_RENDERER` or
`GDK_BACKEND` does **not** work for this failure.

### Debian / Ubuntu (.deb)

```sh
sudo apt install ./bot-hq_<version>_amd64.deb
# or: sudo dpkg -i bot-hq_<version>_amd64.deb
```

The `.deb` declares its `libwebkit2gtk-4.1` dependency. For the AppImage, make
sure a WebKitGTK 4.1 runtime is present (it is on most desktops).

OS notifications (needs-you escalation while the window is unfocused) require
a notification daemon — present on mainstream desktops (GNOME, KDE, XFCE);
minimal window managers may need one installed. **Settings → Notifications →
Send test notification** tells you if sends fail.

## Windows

Run the NSIS installer (`bot-hq_<version>_x64-setup.exe`). The build is
unsigned, so SmartScreen shows "Windows protected your PC" — *More info →
Run anyway*. The git hooks bot-hq installs are bash scripts, so they need Git
for Windows (its bundled bash) on `PATH`; see `docs/WINDOWS-TESTING.md` for
the tester notes.

## Data location

bot-hq stores everything under `~/.bot-hq/` (Context Library, sessions, config,
logs). It is preserved across upgrades and is **not** removed on uninstall.
