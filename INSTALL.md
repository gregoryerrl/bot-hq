# Installing bot-hq

bot-hq ships for **macOS** and **Linux**. (Windows is in progress — see
[PLAN.md](PLAN.md).) Builds are on the
[Releases page](https://github.com/gregoryerrl/bot-hq/releases).

## Prerequisite (all platforms)

bot-hq drives AI agents by running the **`claude-code` CLI** as a subprocess, so
it must be installed and authenticated first:
<https://docs.claude.com/en/docs/claude-code>. `git` is also required for the
repositories you point bot-hq at.

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

### Debian / Ubuntu (.deb)

```sh
sudo apt install ./bot-hq_<version>_amd64.deb
# or: sudo dpkg -i bot-hq_<version>_amd64.deb
```

The `.deb` declares its `libwebkit2gtk-4.1` dependency. For the AppImage, make
sure a WebKitGTK 4.1 runtime is present (it is on most desktops).

## Data location

bot-hq stores everything under `~/.bot-hq/` (Context Library, sessions, config,
logs). It is preserved across upgrades and is **not** removed on uninstall.
