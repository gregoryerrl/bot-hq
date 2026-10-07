//! The terminal commands that sign a second Claude account in to its own
//! config dir and, optionally, share the default dir's user config with it —
//! shown in the Model dialog (`tauri_cmd::models::account_setup_commands`) and
//! in a spawn refusal (`agents::spawn::login_command`). One generator per
//! shell, picked by the platform bot-hq runs on, since the user pastes into a
//! terminal on the same machine: POSIX sh on macOS and Linux, Windows
//! PowerShell 5.1 (the Windows 10/11 default) on Windows.
//!
//! PowerShell 5.1 shapes the Windows half: it has no `&&`; `Remove-Item
//! -Recurse` on a folder link deletes the TARGET's contents, so nothing here
//! deletes anything; `New-Item -ItemType SymbolicLink` needs an administrator
//! window even with Developer Mode on, so folders link as junctions (no
//! privilege needed) and the two files through `cmd /c mklink` (Developer Mode
//! is enough); and a `$env:` assignment outlives the line, so the sign-in
//! restores the previous value in a `finally`, which also runs on Ctrl+C.
//! Every command runs in its own scope (`& { … }` / `( … )`), so none of its
//! variables overwrite one the user already has.

use serde::Serialize;
use specta::Type;

/// The shell a command is written for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum Shell {
    Sh,
    Powershell,
}

impl Shell {
    /// The shell of this machine's terminal.
    pub fn host() -> Self {
        if cfg!(windows) {
            Shell::Powershell
        } else {
            Shell::Sh
        }
    }

    /// Where a command is to be run, as the copy names it.
    pub fn terminal(self) -> &'static str {
        match self {
            Shell::Sh => "your terminal",
            Shell::Powershell => "PowerShell",
        }
    }
}

/// The default dir's user config a second account can share by link, in the
/// order the share step walks it. Never `projects/`: session history and
/// auto-memory stay per account.
pub const SHARED_ITEMS: [&str; 6] = ["CLAUDE.md", "settings.json", "agents", "commands", "skills", "plugins"];

/// The Model dialog's one-time commands for one config dir.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct AccountSetupCommands {
    /// The shell both commands are written for.
    pub shell: Shell,
    /// Create the dir, then sign the account in to it.
    pub setup: String,
    /// Optional: link the default dir's user config into it.
    pub share: String,
}

/// `s` as one literal word in `shell`. PowerShell also treats the curly single
/// quotes as quote characters, so all four are doubled, not only `'`.
pub fn quote(shell: Shell, s: &str) -> String {
    match shell {
        Shell::Sh => format!("'{}'", s.replace('\'', r"'\''")),
        Shell::Powershell => {
            let mut out = String::with_capacity(s.len() + 2);
            out.push('\'');
            for c in s.chars() {
                if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
                    out.push(c);
                }
                out.push(c);
            }
            out.push('\'');
            out
        }
    }
}

/// Sign the account in to `dir`, which exists — the spawn refusal's command.
pub fn login_command(shell: Shell, dir: &str) -> String {
    match shell {
        Shell::Sh => format!("CLAUDE_CONFIG_DIR={} claude auth login --claudeai", quote(shell, dir)),
        Shell::Powershell => format!("& {{ {} }}", powershell_login(dir)),
    }
}

/// The PowerShell sign-in, unscoped: the env var is process-wide, so it is put
/// back (or removed, when `$p` is `$null`) in a `finally`.
fn powershell_login(dir: &str) -> String {
    format!(
        "$p = $env:CLAUDE_CONFIG_DIR; try {{ $env:CLAUDE_CONFIG_DIR = {}; claude auth login --claudeai }} \
         finally {{ $env:CLAUDE_CONFIG_DIR = $p }}",
        quote(Shell::Powershell, dir)
    )
}

/// Create `dir`, then sign the account in to it — the Model dialog's step.
pub fn setup_command(shell: Shell, dir: &str) -> String {
    match shell {
        Shell::Sh => format!("mkdir -p {} && {}", quote(shell, dir), login_command(shell, dir)),
        // `-ErrorAction Stop` makes a failed create end the block before the
        // sign-in, which is what `&&` does in sh.
        Shell::Powershell => format!(
            "& {{ $null = New-Item -ItemType Directory -Force -Path {} -ErrorAction Stop; {} }}",
            quote(shell, dir),
            powershell_login(dir)
        ),
    }
}

/// Link `default_dir`'s shared items into `dir`. An item missing from the
/// default dir is skipped; an item already a link in `dir` — dangling or not —
/// is skipped, so a re-run changes nothing; a real file or folder in `dir` is
/// moved to `<item>.bak` first, unless that `.bak` exists too (then the item is
/// skipped with a warning: nothing is overwritten or nested). When the link
/// then fails — on Windows a file link without Developer Mode or an
/// administrator window — the moved item is put back, so the account is never
/// left without its own settings.json until someone notices.
pub fn share_script(shell: Shell, dir: &str, default_dir: &str) -> String {
    match shell {
        Shell::Sh => [
            "(".to_string(),
            format!("B={}", quote(shell, dir)),
            format!("S={}", quote(shell, default_dir)),
            format!("for item in {}; do", SHARED_ITEMS.join(" ")),
            "  src=\"$S/$item\"; dst=\"$B/$item\"".to_string(),
            "  [ -e \"$src\" ] || continue".to_string(),
            "  [ -L \"$dst\" ] && continue".to_string(),
            "  moved=".to_string(),
            "  if [ -e \"$dst\" ]; then".to_string(),
            "    if [ -e \"$dst.bak\" ] || [ -L \"$dst.bak\" ]; then echo \"skipped $item: $dst.bak already exists\" >&2; continue; fi".to_string(),
            "    mv \"$dst\" \"$dst.bak\" || continue".to_string(),
            "    moved=1".to_string(),
            "  fi".to_string(),
            "  if ! ln -s \"$src\" \"$dst\"; then".to_string(),
            "    if [ -n \"$moved\" ]; then mv \"$dst.bak\" \"$dst\"; fi".to_string(),
            "    echo \"could not link $item\" >&2".to_string(),
            "  fi".to_string(),
            "done".to_string(),
            ")".to_string(),
        ]
        .join("\n"),
        // `[IO.File]::GetAttributes` does not follow a link, so a dangling
        // one is seen as a link (Test-Path would call it absent and the link
        // step would then fail on "already exists").
        Shell::Powershell => {
            let items = SHARED_ITEMS.iter().map(|i| quote(shell, i)).collect::<Vec<_>>().join(", ");
            [
                "& {".to_string(),
                format!("  $B = {}", quote(shell, dir)),
                format!("  $S = {}", quote(shell, default_dir)),
                format!("  foreach ($item in {items}) {{"),
                "    $src = Join-Path $S $item; $dst = Join-Path $B $item".to_string(),
                "    if (-not (Test-Path -LiteralPath $src)) { continue }".to_string(),
                "    $at = $null; try { $at = [IO.File]::GetAttributes($dst) } catch { }".to_string(),
                "    if ($null -ne $at -and ($at -band [IO.FileAttributes]::ReparsePoint)) { continue }".to_string(),
                "    $bak = \"$dst.bak\"; $moved = $false".to_string(),
                "    if ($null -ne $at) {".to_string(),
                "      $bat = $null; try { $bat = [IO.File]::GetAttributes($bak) } catch { }".to_string(),
                "      if ($null -ne $bat) { Write-Warning \"skipped ${item}: $bak already exists\"; continue }".to_string(),
                "      try { Move-Item -LiteralPath $dst -Destination $bak -ErrorAction Stop; $moved = $true } catch { Write-Warning \"skipped ${item}: $_\"; continue }".to_string(),
                "    }".to_string(),
                "    if (Test-Path -LiteralPath $src -PathType Container) {".to_string(),
                "      try { $null = New-Item -ItemType Junction -Path $dst -Target $src -ErrorAction Stop; $linked = $true } catch { $linked = $false }".to_string(),
                "    } else {".to_string(),
                "      cmd /c mklink \"$dst\" \"$src\" | Out-Null".to_string(),
                "      $linked = $LASTEXITCODE -eq 0".to_string(),
                "    }".to_string(),
                "    if (-not $linked) {".to_string(),
                "      if ($moved) { Move-Item -LiteralPath $bak -Destination $dst }".to_string(),
                "      Write-Warning \"could not link ${item}: a file link needs Developer Mode (search 'For developers' in Settings) or an administrator PowerShell; run this again after\"".to_string(),
                "    }".to_string(),
                "  }".to_string(),
                "}".to_string(),
            ]
            .join("\n")
        }
    }
}

/// Both of the Model dialog's commands for `dir`.
pub fn commands(shell: Shell, dir: &str, default_dir: &str) -> AccountSetupCommands {
    AccountSetupCommands {
        shell,
        setup: setup_command(shell, dir),
        share: share_script(shell, dir, default_dir),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sh_commands_are_the_shipped_ones_with_the_dir_quoted() {
        let dir = "/Users/me/.claude-acct-2";
        assert_eq!(
            login_command(Shell::Sh, dir),
            "CLAUDE_CONFIG_DIR='/Users/me/.claude-acct-2' claude auth login --claudeai"
        );
        assert_eq!(
            setup_command(Shell::Sh, dir),
            "mkdir -p '/Users/me/.claude-acct-2' && CLAUDE_CONFIG_DIR='/Users/me/.claude-acct-2' claude auth login --claudeai"
        );
        let share = share_script(Shell::Sh, dir, "/Users/me/.claude");
        assert!(share.starts_with("(\nB='/Users/me/.claude-acct-2'\nS='/Users/me/.claude'\n"), "{share}");
        assert!(share.contains("for item in CLAUDE.md settings.json agents commands skills plugins; do"));
        assert!(share.ends_with("\ndone\n)"), "{share}");
        assert!(!share.contains("projects"), "history and memory are never shared");
        assert!(!share.contains("rm "), "the share step never deletes");
        assert!(share.contains("if [ -n \"$moved\" ]; then mv \"$dst.bak\" \"$dst\"; fi"), "{share}");
    }

    #[test]
    fn powershell_commands_restore_the_env_and_never_delete() {
        let dir = r"C:\Users\me\.claude-acct-2";
        let login = login_command(Shell::Powershell, dir);
        assert_eq!(
            login,
            r"& { $p = $env:CLAUDE_CONFIG_DIR; try { $env:CLAUDE_CONFIG_DIR = 'C:\Users\me\.claude-acct-2'; claude auth login --claudeai } finally { $env:CLAUDE_CONFIG_DIR = $p } }"
        );
        assert_eq!(
            setup_command(Shell::Powershell, dir),
            r"& { $null = New-Item -ItemType Directory -Force -Path 'C:\Users\me\.claude-acct-2' -ErrorAction Stop; $p = $env:CLAUDE_CONFIG_DIR; try { $env:CLAUDE_CONFIG_DIR = 'C:\Users\me\.claude-acct-2'; claude auth login --claudeai } finally { $env:CLAUDE_CONFIG_DIR = $p } }"
        );
        assert!(!login.contains("&&"), "Windows PowerShell 5.1 has no &&");
        let share = share_script(Shell::Powershell, dir, r"C:\Users\me\.claude");
        assert!(share.starts_with("& {\n  $B = 'C:\\Users\\me\\.claude-acct-2'\n  $S = 'C:\\Users\\me\\.claude'\n"), "{share}");
        assert!(share.contains("foreach ($item in 'CLAUDE.md', 'settings.json', 'agents', 'commands', 'skills', 'plugins')"));
        assert!(share.contains("New-Item -ItemType Junction"), "folders link without privilege");
        assert!(share.contains("cmd /c mklink"), "files link with Developer Mode");
        assert!(share.contains("[IO.FileAttributes]::ReparsePoint"), "an existing link is skipped");
        assert!(
            share.contains("if ($moved) { Move-Item -LiteralPath $bak -Destination $dst }"),
            "a failed link puts the moved item back"
        );
        assert!(share.contains("search 'For developers' in Settings"), "Windows 10 and 11 both find it");
        for forbidden in ["Remove-Item", "-Recurse", "SymbolicLink", "&&", "projects"] {
            assert!(!share.contains(forbidden), "{forbidden} must not appear: {share}");
        }
    }

    #[test]
    fn quoting_keeps_an_apostrophe_inside_the_word() {
        assert_eq!(quote(Shell::Sh, "/Users/o'brien/x"), r"'/Users/o'\''brien/x'");
        assert_eq!(quote(Shell::Powershell, r"C:\Users\O'Brien"), r"'C:\Users\O''Brien'");
        // PowerShell ends a single-quoted string at a curly quote as well.
        assert_eq!(
            quote(Shell::Powershell, "C:\\Users\\O\u{2019}Brien\u{2018}\u{201A}\u{201B}"),
            "'C:\\Users\\O\u{2019}\u{2019}Brien\u{2018}\u{2018}\u{201A}\u{201A}\u{201B}\u{201B}'"
        );
        assert!(setup_command(Shell::Powershell, r"C:\Users\O'Brien\acct").contains(r"'C:\Users\O''Brien\acct'"));
        assert!(share_script(Shell::Sh, "/a/it's", "/b").contains(r"B='/a/it'\''s'"));
    }

    #[test]
    fn the_host_shell_is_the_platforms() {
        assert_eq!(Shell::host(), if cfg!(windows) { Shell::Powershell } else { Shell::Sh });
        assert_eq!(Shell::Powershell.terminal(), "PowerShell");
        assert_eq!(Shell::Sh.terminal(), "your terminal");
    }

    /// The fixture both executing share tests use: a "home" whose path has a
    /// space and an apostrophe (Windows user names carry both), a default dir
    /// holding four of the six shared items plus `projects/`, and a second
    /// account's dir that already has a real CLAUDE.md (→ `.bak`), a real
    /// settings.json whose `.bak` also exists (→ skipped, nothing overwritten),
    /// and a dangling link at `commands` (→ skipped, nothing deleted).
    struct ShareFixture {
        _root: tempfile::TempDir,
        default_dir: std::path::PathBuf,
        dir: std::path::PathBuf,
    }

    fn share_fixture(dangling: impl FnOnce(&std::path::Path, &std::path::Path)) -> ShareFixture {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("it's home");
        let default_dir = home.join(".claude");
        let dir = home.join(".claude acct 2");
        std::fs::create_dir_all(default_dir.join("skills")).unwrap();
        std::fs::create_dir_all(default_dir.join("commands")).unwrap();
        std::fs::create_dir_all(default_dir.join("projects").join("p")).unwrap();
        std::fs::write(default_dir.join("CLAUDE.md"), "shared rules").unwrap();
        std::fs::write(default_dir.join("settings.json"), "{\"shared\":true}").unwrap();
        std::fs::write(default_dir.join("skills").join("a.md"), "skill").unwrap();
        std::fs::write(default_dir.join("commands").join("c.md"), "command").unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("CLAUDE.md"), "account 2's own").unwrap();
        std::fs::write(dir.join("settings.json"), "{\"mine\":true}").unwrap();
        std::fs::write(dir.join("settings.json.bak"), "{\"older\":true}").unwrap();
        let gone = root.path().join("gone");
        std::fs::create_dir_all(&gone).unwrap();
        dangling(&gone, &dir.join("commands"));
        std::fs::remove_dir_all(&gone).unwrap();
        ShareFixture { _root: root, default_dir, dir }
    }

    /// Every entry of the account dir: a link's target, a file's content, or a
    /// folder marker — the state a re-run must leave exactly as it was.
    fn describe(dir: &std::path::Path) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                let name = e.file_name().to_string_lossy().into_owned();
                let meta = std::fs::symlink_metadata(e.path()).unwrap();
                let what = if meta.file_type().is_symlink() {
                    format!("link -> {}", std::fs::read_link(e.path()).unwrap().display())
                } else if meta.is_dir() {
                    "dir".to_string()
                } else {
                    format!("file: {}", std::fs::read_to_string(e.path()).unwrap())
                };
                (name, what)
            })
            .collect();
        out.sort();
        out
    }

    fn assert_shared(f: &ShareFixture) {
        let link = |name: &str| std::fs::symlink_metadata(f.dir.join(name)).unwrap().file_type().is_symlink();
        // Linked: present in the default dir, absent or real in the account's.
        assert!(link("CLAUDE.md") && link("skills"), "{:?}", describe(&f.dir));
        assert_eq!(std::fs::read_to_string(f.dir.join("CLAUDE.md")).unwrap(), "shared rules");
        assert_eq!(std::fs::read_to_string(f.dir.join("skills").join("a.md")).unwrap(), "skill");
        assert_eq!(std::fs::read_to_string(f.dir.join("CLAUDE.md.bak")).unwrap(), "account 2's own");
        // Skipped: a real item whose .bak exists, untouched; nothing overwritten.
        assert!(!link("settings.json"));
        assert_eq!(std::fs::read_to_string(f.dir.join("settings.json")).unwrap(), "{\"mine\":true}");
        assert_eq!(std::fs::read_to_string(f.dir.join("settings.json.bak")).unwrap(), "{\"older\":true}");
        // Skipped: the dangling link stays a link; the source it shadows is intact.
        assert!(link("commands"));
        assert_eq!(std::fs::read_to_string(f.default_dir.join("commands").join("c.md")).unwrap(), "command");
        // Absent from the default dir, or never shared.
        for name in ["agents", "plugins", "projects"] {
            assert!(!f.dir.join(name).exists(), "{name} must not be linked: {:?}", describe(&f.dir));
        }
    }

    /// The POSIX shells the sh commands are pasted into: `/bin/sh` (dash on
    /// ubuntu CI, bash-as-sh on macOS) and zsh, macOS's default terminal shell
    /// (`-f`: no startup files), where it exists.
    #[cfg(unix)]
    fn posix_shells() -> Vec<[&'static str; 2]> {
        [["/bin/sh", "-c"], ["/bin/zsh", "-fc"]]
            .into_iter()
            .filter(|[shell, _]| std::path::Path::new(shell).exists())
            .collect()
    }

    #[cfg(unix)]
    fn run_posix(shell: [&str; 2], script: &str, envs: &[(&str, &str)]) -> std::process::Output {
        let mut cmd = std::process::Command::new(shell[0]);
        cmd.arg(shell[1]).arg(script).env_clear().env("PATH", "/usr/bin:/bin");
        for (k, v) in envs {
            cmd.env(k, v);
        }
        cmd.output().unwrap()
    }

    /// An executable stub named `name` in `bin`.
    #[cfg(unix)]
    fn stub(bin: &std::path::Path, name: &str, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(bin).unwrap();
        let path = bin.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn the_sh_share_script_links_what_exists_and_a_rerun_changes_nothing() {
        for shell in posix_shells() {
            let f = share_fixture(|target, at| std::os::unix::fs::symlink(target, at).unwrap());
            let script = share_script(Shell::Sh, f.dir.to_str().unwrap(), f.default_dir.to_str().unwrap());
            let out = run_posix(shell, &script, &[]);
            assert!(out.status.success(), "{shell:?}: {}", String::from_utf8_lossy(&out.stderr));
            assert!(String::from_utf8_lossy(&out.stderr).contains("skipped settings.json"), "{shell:?}");
            assert_shared(&f);
            let first = describe(&f.dir);
            let again = run_posix(shell, &script, &[]);
            assert!(again.status.success(), "{shell:?}: {}", String::from_utf8_lossy(&again.stderr));
            assert_eq!(describe(&f.dir), first, "{shell:?}: a re-run changes nothing");
            assert_shared(&f);
        }
    }

    /// A link that fails puts the moved item back (a stub `ln` that always
    /// fails stands in for the Windows privilege failure): the account keeps
    /// its own CLAUDE.md instead of being left with only a `.bak`.
    #[cfg(unix)]
    #[test]
    fn the_sh_share_script_puts_an_item_back_when_its_link_fails() {
        for shell in posix_shells() {
            let f = share_fixture(|target, at| std::os::unix::fs::symlink(target, at).unwrap());
            let bin = f.dir.parent().unwrap().join("bin");
            stub(&bin, "ln", "exit 1");
            let path = format!("{}:/usr/bin:/bin", bin.display());
            let script = share_script(Shell::Sh, f.dir.to_str().unwrap(), f.default_dir.to_str().unwrap());
            let out = run_posix(shell, &script, &[("PATH", path.as_str())]);
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(stderr.contains("could not link CLAUDE.md"), "{shell:?}: {stderr}");
            assert_eq!(std::fs::read_to_string(f.dir.join("CLAUDE.md")).unwrap(), "account 2's own", "{shell:?}");
            assert!(!f.dir.join("CLAUDE.md.bak").exists(), "{shell:?}: {:?}", describe(&f.dir));
            assert!(!f.dir.join("skills").exists(), "{shell:?}");
        }
    }

    /// A stub `claude` first on a PATH that holds nothing else of the user's:
    /// it records the dir it was started with and fails. The real CLI must
    /// never be reachable here — its sign-in would wait on a browser.
    #[cfg(unix)]
    #[test]
    fn the_sh_setup_creates_the_dir_and_signs_in_there_without_touching_the_callers_env() {
        for shell in posix_shells() {
            let root = tempfile::tempdir().unwrap();
            let bin = root.path().join("bin");
            stub(&bin, "claude", "printf '%s' \"$CLAUDE_CONFIG_DIR\" > \"$OUT\"\nexit 1");
            let dir = root.path().join("it's home").join("acct 2");
            let seen = root.path().join("seen");
            let script = format!(
                "{}; echo \"after:${{CLAUDE_CONFIG_DIR-unset}}\"",
                setup_command(Shell::Sh, dir.to_str().unwrap())
            );
            let path = format!("{}:/usr/bin:/bin", bin.display());
            let out = run_posix(shell, &script, &[("PATH", path.as_str()), ("OUT", seen.to_str().unwrap())]);
            assert!(dir.is_dir(), "{shell:?}: the dir is created first");
            assert_eq!(std::fs::read_to_string(&seen).unwrap(), dir.to_str().unwrap(), "{shell:?}");
            assert!(String::from_utf8_lossy(&out.stdout).contains("after:unset"), "{shell:?}");
            let out = run_posix(
                shell,
                &script,
                &[("PATH", path.as_str()), ("OUT", seen.to_str().unwrap()), ("CLAUDE_CONFIG_DIR", "prev")],
            );
            assert!(String::from_utf8_lossy(&out.stdout).contains("after:prev"), "{shell:?}");
        }
    }

    /// Windows PowerShell 5.1 — never `pwsh`: the user's default shell is the
    /// one these commands must work in. `-EncodedCommand` sidesteps argv
    /// quoting; `-ExecutionPolicy Bypass` the machine policy. PATH holds the
    /// stub dir and the system dirs only.
    #[cfg(windows)]
    fn run_powershell(script: &str, stub_dir: Option<&std::path::Path>, envs: &[(&str, &str)]) -> std::process::Output {
        use base64::Engine as _;
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let ps = format!(r"{root}\System32\WindowsPowerShell\v1.0");
        let mut path = format!(r"{root}\System32;{ps}");
        if let Some(stub) = stub_dir {
            path = format!("{};{path}", stub.display());
        }
        let utf16: Vec<u8> = script.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        let encoded = base64::engine::general_purpose::STANDARD.encode(utf16);
        let mut cmd = std::process::Command::new(format!(r"{ps}\powershell.exe"));
        cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand", &encoded])
            .env("PATH", path)
            .env_remove("CLAUDE_CONFIG_DIR");
        for (k, v) in envs {
            cmd.env(k, v);
        }
        cmd.output().unwrap()
    }

    /// The fixture's dangling link, made the way a Windows user would have one:
    /// a junction (no privilege needed), whose target the fixture then removes.
    #[cfg(windows)]
    fn junction(target: &std::path::Path, at: &std::path::Path) {
        let make = format!(
            "$null = New-Item -ItemType Junction -Path {} -Target {}",
            quote(Shell::Powershell, at.to_str().unwrap()),
            quote(Shell::Powershell, target.to_str().unwrap())
        );
        let out = run_powershell(&make, None, &[]);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }

    #[cfg(windows)]
    #[test]
    fn the_powershell_share_script_links_what_exists_and_a_rerun_changes_nothing() {
        let f = share_fixture(junction);
        let script = share_script(Shell::Powershell, f.dir.to_str().unwrap(), f.default_dir.to_str().unwrap());
        let out = run_powershell(&script, None, &[]);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert_shared(&f);
        let first = describe(&f.dir);
        let again = run_powershell(&script, None, &[]);
        assert!(again.status.success(), "{}", String::from_utf8_lossy(&again.stderr));
        assert_eq!(describe(&f.dir), first, "a re-run changes nothing");
        assert_shared(&f);
    }

    /// The branch the elevated CI runner never reaches on its own: `mklink`
    /// refused (a non-admin window without Developer Mode). A `cmd.bat` stub
    /// first on PATH refuses every link; the moved CLAUDE.md must come back,
    /// and the folders — junctions, no `cmd` involved — still link.
    #[cfg(windows)]
    #[test]
    fn the_powershell_share_script_puts_an_item_back_when_its_link_fails() {
        let f = share_fixture(junction);
        let bin = f.dir.parent().unwrap().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("cmd.bat"), "@exit /b 1\r\n").unwrap();
        let script = share_script(Shell::Powershell, f.dir.to_str().unwrap(), f.default_dir.to_str().unwrap());
        let out = run_powershell(&script, Some(&bin), &[]);
        let said = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(said.contains("could not link CLAUDE.md"), "{said}");
        assert_eq!(std::fs::read_to_string(f.dir.join("CLAUDE.md")).unwrap(), "account 2's own");
        assert!(!f.dir.join("CLAUDE.md.bak").exists(), "{:?}", describe(&f.dir));
        assert!(std::fs::symlink_metadata(f.dir.join("skills")).unwrap().file_type().is_symlink());
    }

    #[cfg(windows)]
    #[test]
    fn the_powershell_setup_signs_in_there_and_puts_the_env_back_even_when_stopped() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("claude.cmd"), "@echo off\r\n>\"%OUT%\" echo %CLAUDE_CONFIG_DIR%\r\nexit /b 1\r\n").unwrap();
        let dir = root.path().join("it's home").join("acct 2");
        let seen = root.path().join("seen");
        let setup = setup_command(Shell::Powershell, dir.to_str().unwrap());
        let after = "Write-Output ('after:' + $env:CLAUDE_CONFIG_DIR + '|' + (Test-Path Env:CLAUDE_CONFIG_DIR))";
        let out_env = [("OUT", seen.to_str().unwrap())];

        // The stub exits 1: the dir exists, the stub saw it, the var is back.
        let out = run_powershell(&format!("$env:CLAUDE_CONFIG_DIR = 'prev'; {setup}; {after}"), Some(&bin), &out_env);
        assert!(dir.is_dir(), "the dir is created first: {}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(std::fs::read_to_string(&seen).unwrap().trim_end(), dir.to_str().unwrap());
        assert!(String::from_utf8_lossy(&out.stdout).contains("after:prev|True"), "{}", String::from_utf8_lossy(&out.stdout));
        // Unset before → unset after (not left holding the account's dir).
        let out = run_powershell(&format!("{setup}; {after}"), Some(&bin), &out_env);
        assert!(String::from_utf8_lossy(&out.stdout).contains("after:|False"), "{}", String::from_utf8_lossy(&out.stdout));
        // A sign-in that STOPS the line (what Ctrl+C does): only the `finally`
        // can put the var back. A native non-zero exit would not test this —
        // PowerShell runs the rest of a `;` line after one.
        let login = login_command(Shell::Powershell, dir.to_str().unwrap());
        let out = run_powershell(
            &format!("function claude {{ throw 'stop' }}; $env:CLAUDE_CONFIG_DIR = 'prev'; try {{ {login} }} catch {{ }}; {after}"),
            Some(&bin),
            &out_env,
        );
        assert!(String::from_utf8_lossy(&out.stdout).contains("after:prev|True"), "{}", String::from_utf8_lossy(&out.stdout));
    }
}
