//! The one notification signal that carries information (1.0.1 Batch 6).
//!
//! On desktop the notification plugin's permission API is a compile-time
//! `Granted` constant and `sendNotification` wraps a synchronous constructor
//! returning nothing — so bot-hq cannot tell a user their notifications are
//! off from the send path at all (windows-compat handoff §6; that is how a
//! disabled OS master switch read as a product bug for a full release). On
//! Windows the `ToastEnabled` registry value IS readable, and it belongs
//! beside the user-facing claim (the Settings test button), never in the
//! fire-and-forget send path.

use crate::tauri_cmd::error::AppError;

/// Parse `reg.exe query` output for the ToastEnabled DWORD.
///
/// Uncfg'd on purpose (the windows-compat lesson: cfg'd logic never even
/// type-checks off-platform) — the pure parser runs in every platform's test
/// suite; only the `reg.exe` spawn is Windows-gated.
///
/// `Some(false)` = explicitly 0 (the OS master switch is off — no app can
/// toast); `Some(true)` = any other value; `None` = the value line is absent
/// (callers treat that as enabled: Windows' default state stores no value).
pub(crate) fn parse_toast_enabled(reg_stdout: &str) -> Option<bool> {
    let line = reg_stdout.lines().find(|l| l.contains("ToastEnabled"))?;
    let raw = line.split_whitespace().last()?;
    let v = u32::from_str_radix(raw.trim_start_matches("0x"), 16).ok()?;
    Some(v != 0)
}

#[cfg(windows)]
fn read_toast_enabled() -> Option<bool> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000; // a bare reg.exe flashes a console
    let out = std::process::Command::new("reg")
        .args([
            "query",
            r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\PushNotifications",
            "/v",
            "ToastEnabled",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !out.status.success() {
        // Key or value absent — Windows stores no value in its default
        // (enabled) state, so absence is not a warning.
        return Some(true);
    }
    parse_toast_enabled(&String::from_utf8_lossy(&out.stdout)).or(Some(true))
}

/// `Some(false)` = Windows says no app may toast (surface a warning beside
/// the test button). `Some(true)` = the OS switch is on. `None` = not
/// Windows — the signal does not exist elsewhere; say nothing.
#[tauri::command]
#[specta::specta]
pub async fn windows_toast_enabled() -> Result<Option<bool>, AppError> {
    #[cfg(windows)]
    {
        Ok(tokio::task::spawn_blocking(read_toast_enabled)
            .await
            .unwrap_or(None))
    }
    #[cfg(not(windows))]
    {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::parse_toast_enabled;

    #[test]
    fn the_registry_parse_reads_the_three_states() {
        // The shape `reg query` prints (measured on the windows-compat box).
        let disabled = "\r\nHKEY_CURRENT_USER\\...\\PushNotifications\r\n    ToastEnabled    REG_DWORD    0x0\r\n";
        assert_eq!(parse_toast_enabled(disabled), Some(false));
        let enabled = "    ToastEnabled    REG_DWORD    0x1";
        assert_eq!(parse_toast_enabled(enabled), Some(true));
        // Absent value line → None (callers read that as default-enabled).
        assert_eq!(parse_toast_enabled("ERROR: The system was unable to find the specified registry key or value."), None);
        // Garbage after the name must not panic and must not claim disabled.
        assert_eq!(parse_toast_enabled("    ToastEnabled    REG_DWORD    zz"), None);
    }
}
