//! Where a gate hook's time went (feedback #121).
//!
//! The Codex PreToolUse hook (`policy-check codex-gate`) has a 30 s budget,
//! and when Codex gave up on one the only trace was `hook timed out after
//! 30s`: the hook had written nothing (its messages are terminal
//! `eprintln!`s), the host saw `hook/completed` with `status: failed`, and
//! which phase — reading the payload, the posture HTTP call, a policy file,
//! parking the gate — had stalled was anyone's guess. So the hook keeps a
//! BREADCRUMB RECORD while it runs: one small JSON file per invocation under
//! `<data_dir>/.local/hook-phases/<session>/<agent>/<start-ms>-<pid>.json`,
//! rewritten at every phase boundary and removed on exit. A hook that exits
//! leaves nothing; a hook that was killed leaves the phase it was in, with
//! the time it entered it. Writing it costs a few microseconds and never
//! decides anything: the file is diagnostics, the hook's exit code is the
//! gate.
//!
//! The host reads the record back when Codex reports the failed run
//! ([`diagnose`]). The ONE identifier both sides share is the turn id
//! (`turn_id` in the hook's stdin payload, `turnId` on the notification);
//! Codex's run id reaches neither the hook's stdin nor its environment. So
//! a record is matched by the turn id when both have one AND by time —
//! Codex's `startedAt` must precede the hook process's own start within a
//! small slack, and the record must not be older than the run. Exactly one
//! candidate names the phase; none or several says so instead of guessing
//! (eyes-2: an ambiguous or stale match must read as unknown), and a run
//! with no record at all means the hook binary never reached its first line
//! — the shell startup / exec in front of it is where that time went.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A record this much older than the run it is compared with is a leftover
/// from an earlier kill nobody consumed, and is deleted.
pub const STALE_AFTER_MS: i64 = 15 * 60 * 1000;
/// How much earlier than Codex's `startedAt` a hook process may have started
/// and still belong to that run — the two stamps come from one machine's
/// clock, but are taken at different points of the spawn.
pub const START_SLACK_MS: i64 = 2_000;
/// With no `startedAt` on the notification, the window a record may fall in.
const UNSTAMPED_WINDOW_MS: i64 = 120_000;

/// The directory of one participant's records, or `None` when a name is not
/// a safe path segment (then there is no record and no diagnosis).
pub fn dir_under(local_dir: &Path, session_id: &str, agent: &str) -> Option<PathBuf> {
    (crate::agents::handoff::safe_segment(session_id) && crate::agents::handoff::safe_segment(agent))
        .then(|| local_dir.join("hook-phases").join(session_id).join(agent))
}

/// Milliseconds since the Unix epoch, the stamp both sides compare.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Phase {
    pub phase: String,
    pub at_ms: i64,
}

/// One hook invocation's breadcrumbs. No command text: the record names the
/// tool and the call, never what it asked to run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HookPhaseRecord {
    pub version: u32,
    pub hook: String,
    pub pid: u32,
    pub started_at_ms: i64,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub tool_use_id: Option<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
    pub phases: Vec<Phase>,
}

/// The hook side: `begin` writes the record before anything can block,
/// `enter` marks each phase, `finish` removes it. Every write failure is
/// swallowed — a diagnostic must never change what the hook decides.
pub struct PhaseRecorder {
    path: Option<PathBuf>,
    record: HookPhaseRecord,
}

impl PhaseRecorder {
    pub fn begin(local_dir: &Path, hook: &str, session_id: Option<&str>, agent: &str) -> Self {
        let now = now_ms();
        let record = HookPhaseRecord {
            version: 1,
            hook: hook.to_string(),
            pid: std::process::id(),
            started_at_ms: now,
            turn_id: None,
            tool_use_id: None,
            tool_name: None,
            phases: Vec::new(),
        };
        let path = session_id.and_then(|sid| dir_under(local_dir, sid, agent)).and_then(|dir| {
            std::fs::create_dir_all(&dir).ok()?;
            remove_stale(&dir, now);
            Some(dir.join(format!("{now}-{}.json", record.pid)))
        });
        let recorder = Self { path, record };
        recorder.write();
        recorder
    }

    /// The hook entered `phase` now.
    pub fn enter(&mut self, phase: &str) {
        self.record.phases.push(Phase { phase: phase.to_string(), at_ms: now_ms() });
        self.write();
    }

    /// What the payload said about the call, once it has been read.
    pub fn identify(&mut self, turn_id: Option<&str>, tool_use_id: Option<&str>, tool_name: Option<&str>) {
        self.record.turn_id = turn_id.map(str::to_string);
        self.record.tool_use_id = tool_use_id.map(str::to_string);
        self.record.tool_name = tool_name.map(str::to_string);
        self.write();
    }

    /// The hook is exiting on its own: nothing to diagnose.
    pub fn finish(self) {
        if let Some(path) = &self.path {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Where the record is kept (tests).
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    fn write(&self) {
        let Some(path) = &self.path else { return };
        let Ok(body) = serde_json::to_vec(&self.record) else { return };
        // Whole file or nothing: a reader never sees a half-written record.
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, body).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

fn remove_stale(dir: &Path, now: i64) {
    for (path, record) in records_in(dir) {
        if now - record.started_at_ms > STALE_AFTER_MS {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn records_in(dir: &Path) -> Vec<(PathBuf, HookPhaseRecord)> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<(PathBuf, HookPhaseRecord)> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| {
            let record = serde_json::from_slice(&std::fs::read(&p).ok()?).ok()?;
            Some((p, record))
        })
        .collect();
    out.sort_by_key(|(_, r)| r.started_at_ms);
    out
}

/// Codex's `startedAt` / `completedAt` as epoch milliseconds. The schema says
/// only `int64`; the magnitude says the unit — seconds (today ~1.8e9),
/// milliseconds (~1.8e12), microseconds (~1.8e15) or nanoseconds (~1.8e18)
/// — so a unit the schema leaves open cannot push a record out of the window.
fn epoch_ms(v: &serde_json::Value) -> Option<i64> {
    let n = v.as_i64()?;
    if n <= 0 {
        None
    } else if n < 100_000_000_000 {
        Some(n * 1000)
    } else if n < 100_000_000_000_000 {
        Some(n)
    } else if n < 100_000_000_000_000_000 {
        Some(n / 1000)
    } else {
        Some(n / 1_000_000)
    }
}

fn span(ms: i64) -> String {
    if ms < 1000 { format!("{ms} ms") } else { format!("{:.1} s", ms as f64 / 1000.0) }
}

/// Whether a process with the record's pid is still running: `Some(true)`
/// yes, `Some(false)` no, `None` where bot-hq cannot ask (not Unix, or the
/// check itself failed). An OBSERVATION for the notice, never a
/// discriminator: a live pid may be another call's hook or a reused number,
/// and a gone one proves nothing about which run Codex gave up on.
fn process_alive(pid: u32) -> Option<bool> {
    #[cfg(unix)]
    {
        // SAFETY: kill(2) with signal 0 only checks; a stale pid answers ESRCH.
        let answered = unsafe { libc::kill(pid as i32, 0) };
        if answered == 0 {
            return Some(true);
        }
        match std::io::Error::last_os_error().raw_os_error() {
            Some(libc::ESRCH) => Some(false),
            Some(libc::EPERM) => Some(true),
            _ => None,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        None
    }
}

/// The host side: the sentence to append to the FAILED-hook notice for the
/// run Codex reported (`params.run`, `params.turnId`). Reads the participant's
/// records (sweeping stale ones) and never attributes: the two sides carry no
/// shared invocation id (eyes-2 `f4a9c42d` / `27d5ffc7`), so a record is only
/// ever "overlapping" — same turn when both have one, start inside the run's
/// window. One overlapping record is shown with its recorded phases and
/// ownership unconfirmed; several are listed and the owner is unknown; none
/// is missing evidence (the hook may not have started in the window, or wrote
/// nothing, or already cleaned up). Whether a process with the record's pid
/// still runs is reported beside each as an observation. Records are never
/// deleted here: a live hook finishes its own, a leftover goes stale.
pub fn diagnose(
    local_dir: &Path,
    session_id: &str,
    agent: &str,
    run: &serde_json::Value,
    turn_id: Option<&str>,
    now: i64,
) -> String {
    diagnose_with(local_dir, session_id, agent, run, turn_id, now, process_alive)
}

pub fn diagnose_with(
    local_dir: &Path,
    session_id: &str,
    agent: &str,
    run: &serde_json::Value,
    turn_id: Option<&str>,
    now: i64,
    alive: impl Fn(u32) -> Option<bool>,
) -> String {
    let Some(dir) = dir_under(local_dir, session_id, agent) else {
        return "No hook record can exist for this participant (its names are not usable as paths), so the stalled phase is unknown.".to_string();
    };
    let started = epoch_ms(&run["startedAt"]);
    let duration = run["durationMs"].as_i64().filter(|d| *d >= 0);
    let completed = epoch_ms(&run["completedAt"])
        .or_else(|| Some(started? + duration?))
        .unwrap_or(now)
        .min(now.max(started.unwrap_or(now)));
    let window_start = started.map_or(completed - UNSTAMPED_WINDOW_MS, |s| s - START_SLACK_MS);
    let run_text = match (started, duration) {
        (Some(_), Some(d)) => format!("Codex gave the hook up after {}", span(d)),
        (Some(_), None) => "Codex gave the hook up".to_string(),
        (None, Some(d)) => format!("Codex gave the hook up after {} (the run carried no start stamp)", span(d)),
        (None, None) => "Codex gave the hook up (the run carried no stamps)".to_string(),
    };

    let mut overlapping: Vec<(HookPhaseRecord, Option<bool>)> = Vec::new();
    for (path, record) in records_in(&dir) {
        if completed - record.started_at_ms > STALE_AFTER_MS {
            let _ = std::fs::remove_file(path);
            continue;
        }
        let same_turn = match (turn_id, record.turn_id.as_deref()) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        };
        if same_turn && (window_start..=completed).contains(&record.started_at_ms) {
            let liveness = alive(record.pid);
            overlapping.push((record, liveness));
        }
    }
    // One line per record: where it started relative to the run, the last
    // phase it recorded (what was written, not proof of where it stayed) and
    // what its pid says now.
    let brief = |r: &HookPhaseRecord, liveness: Option<bool>| {
        let last = r.phases.last().map_or("no phase recorded".to_string(), |p| format!("last recorded phase {}", p.phase));
        let when = started.map_or(String::new(), |s| format!(" started +{}", span((r.started_at_ms - s).max(0))));
        let pid = match liveness {
            Some(false) => "its process is gone",
            Some(true) => "a process with its pid is still running, so it may be another call's hook",
            None => "its process state could not be checked",
        };
        match &r.turn_id {
            Some(t) => format!("pid {}{when}, turn {t}, {last}, {pid}", r.pid),
            None => format!("pid {}{when}, {last}, {pid}", r.pid),
        }
    };
    match overlapping.len() {
        0 => format!(
            "{run_text}; no bot-hq hook record overlaps that run — the hook process may not have started in that \
             window, or its record was never written or already removed: the evidence is missing, so the stalled \
             phase is unknown."
        ),
        1 => {
            let (record, liveness) = &overlapping[0];
            let mut text = format!(
                "{run_text}; one bot-hq hook record overlaps that run (ownership unconfirmed: the two carry no \
                 shared invocation id) — {}",
                brief(record, *liveness)
            );
            if record.phases.is_empty() {
                text.push_str(" (killed before reading the payload, or its writes failed)");
            } else {
                text.push_str("; recorded phases ");
                let mut steps = Vec::new();
                for (i, phase) in record.phases.iter().enumerate() {
                    match record.phases.get(i + 1) {
                        Some(next) => steps.push(format!("{} {}", phase.phase, span((next.at_ms - phase.at_ms).max(0)))),
                        None => steps.push(format!("{} entered {} before Codex gave up", phase.phase, span((completed - phase.at_ms).max(0)))),
                    }
                }
                text.push_str(&steps.join(" → "));
            }
            text.push_str("; the record is kept until it goes stale.");
            text
        }
        n => format!(
            "{run_text}; {n} bot-hq hook records overlap that run without a shared invocation id ({}), so which one \
             Codex gave up on is unknown.",
            overlapping.iter().map(|(r, l)| brief(r, *l)).collect::<Vec<_>>().join("; ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn local() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    /// The hook's lifecycle: the record exists from `begin`, grows a phase at
    /// a time, carries what the payload said, and is gone after `finish`.
    #[test]
    fn a_record_lives_exactly_as_long_as_the_hook_runs() {
        let local = local();
        let mut rec = PhaseRecorder::begin(local.path(), "codex-gate", Some("s-1"), "hands");
        let path = rec.path().expect("a safe pair gets a record").to_path_buf();
        assert!(path.exists(), "written before anything can block");
        assert!(path.starts_with(local.path().join("hook-phases").join("s-1").join("hands")));
        rec.enter("read-payload");
        rec.identify(Some("t1"), Some("call_1"), Some("Bash"));
        rec.enter("fetch-posture");
        let on_disk: HookPhaseRecord = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(on_disk.pid, std::process::id());
        assert_eq!(on_disk.turn_id.as_deref(), Some("t1"));
        assert_eq!(on_disk.tool_use_id.as_deref(), Some("call_1"));
        assert_eq!(on_disk.tool_name.as_deref(), Some("Bash"));
        assert_eq!(on_disk.phases.iter().map(|p| p.phase.as_str()).collect::<Vec<_>>(), ["read-payload", "fetch-posture"]);
        assert!(!std::fs::read_to_string(&path).unwrap().contains("command"), "no command text in a record");
        rec.finish();
        assert!(!path.exists(), "a hook that exits leaves nothing");
        // No session id, or an unsafe name: inert, never a panic.
        let none = PhaseRecorder::begin(local.path(), "codex-gate", None, "hands");
        assert!(none.path().is_none());
        let unsafe_name = PhaseRecorder::begin(local.path(), "codex-gate", Some("../x"), "hands");
        assert!(unsafe_name.path().is_none());
    }

    fn write_record(dir: &Path, started_at_ms: i64, pid: u32, turn: Option<&str>, phases: &[(&str, i64)]) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let record = HookPhaseRecord {
            version: 1,
            hook: "codex-gate".into(),
            pid,
            started_at_ms,
            turn_id: turn.map(str::to_string),
            tool_use_id: Some(format!("call_{pid}")),
            tool_name: Some("Bash".into()),
            phases: phases.iter().map(|(p, at)| Phase { phase: p.to_string(), at_ms: *at }).collect(),
        };
        let path = dir.join(format!("{started_at_ms}-{pid}.json"));
        std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
        path
    }

    /// Liveness oracles for the tests: the answer is an observation only.
    fn gone(_: u32) -> Option<bool> { Some(false) }
    fn alive(_: u32) -> Option<bool> { Some(true) }

    /// One record in the run's window and turn: it is shown — startup gap,
    /// every recorded phase's time, the last recorded phase, what its pid says
    /// — with ownership unconfirmed, and it is kept for the stale sweep.
    #[test]
    fn a_single_overlapping_record_is_shown_without_claiming_ownership() {
        let local = local();
        let dir = dir_under(local.path(), "s-1", "hands").unwrap();
        let t0 = 1_761_500_000_000_i64; // Codex's startedAt, epoch ms
        let path = write_record(&dir, t0 + 1_200, 41, Some("t1"), &[
            ("read-payload", t0 + 1_210), ("parse-payload", t0 + 1_212), ("shell-lint", t0 + 1_213),
            ("start-runtime", t0 + 1_216), ("fetch-posture", t0 + 1_300),
        ]);
        let run = json!({"eventName": "preToolUse", "status": "failed", "startedAt": t0, "durationMs": 30_002, "completedAt": t0 + 30_002});
        let text = diagnose_with(local.path(), "s-1", "hands", &run, Some("t1"), t0 + 31_000, gone);
        assert!(text.contains("Codex gave the hook up after 30.0 s"), "{text}");
        assert!(text.contains("one bot-hq hook record overlaps that run (ownership unconfirmed: the two carry no shared invocation id)"), "{text}");
        assert!(text.contains("pid 41 started +1.2 s, turn t1, last recorded phase fetch-posture, its process is gone"), "{text}");
        assert!(text.contains("recorded phases read-payload 2 ms → parse-payload 1 ms → shell-lint 3 ms → start-runtime 84 ms → fetch-posture entered 28.7 s before Codex gave up"), "{text}");
        assert!(text.contains("the record is kept until it goes stale"), "{text}");
        assert!(!text.contains("most likely") && !text.contains("not this run's"), "no attribution: {text}");
        assert!(path.exists(), "a candidate is evidence, never deleted by the reader");
        // The same record read with a live pid, or an uncheckable one: the
        // observation changes, the claim does not.
        let text = diagnose_with(local.path(), "s-1", "hands", &run, Some("t1"), t0 + 31_000, alive);
        assert!(text.contains("a process with its pid is still running, so it may be another call's hook") && text.contains("ownership unconfirmed"), "{text}");
        let text = diagnose_with(local.path(), "s-1", "hands", &run, Some("t1"), t0 + 31_000, |_| None);
        assert!(text.contains("its process state could not be checked") && text.contains("ownership unconfirmed"), "{text}");
    }

    /// eyes-2 `f4a9c42d` / `27d5ffc7`: run A failed before its hook wrote
    /// anything; the same turn's hook B is running with its record in A's
    /// window. B is the only overlap, so it is shown — as an overlap with
    /// ownership unconfirmed, its live pid an observation — never attributed
    /// or deleted. With A's own record beside it, both count: unknown.
    #[test]
    fn overlaps_are_never_attributed_whatever_their_pids_say() {
        let local = local();
        let dir = dir_under(local.path(), "s-1", "hands").unwrap();
        let t0 = 1_761_500_000_000_i64;
        let b = write_record(&dir, t0 + 5_000, 77, Some("t1"), &[("read-payload", t0 + 5_010), ("fetch-posture", t0 + 5_100)]);
        let run = json!({"startedAt": t0, "durationMs": 30_000});
        let text = diagnose_with(local.path(), "s-1", "hands", &run, Some("t1"), t0 + 31_000, alive);
        assert!(text.contains("one bot-hq hook record overlaps that run (ownership unconfirmed"), "{text}");
        assert!(text.contains("pid 77 started +5.0 s, turn t1, last recorded phase fetch-posture, a process with its pid is still running, so it may be another call's hook"), "{text}");
        assert!(!text.contains("most likely") && !text.contains("not this run's"), "{text}");
        assert!(b.exists(), "B's record is untouched");
        // A's own record (process gone) beside B's (running): two overlaps,
        // listed with their observations, owner unknown — liveness ranks nothing.
        let a = write_record(&dir, t0 + 900, 41, Some("t1"), &[("read-payload", t0 + 910)]);
        let text = diagnose_with(local.path(), "s-1", "hands", &run, Some("t1"), t0 + 31_000, |pid| Some(pid == 77));
        assert!(text.contains("2 bot-hq hook records overlap that run without a shared invocation id"), "{text}");
        assert!(text.contains("pid 41 started +900 ms, turn t1, last recorded phase read-payload, its process is gone; pid 77 started +5.0 s, turn t1, last recorded phase fetch-posture, a process with its pid is still running, so it may be another call's hook"), "{text}");
        assert!(text.contains("which one Codex gave up on is unknown"), "{text}");
        assert!(a.exists() && b.exists());
        // The real oracle, on the wire: this test's own pid reads as running,
        // and a pid no system hands out as gone.
        std::fs::remove_file(&b).unwrap();
        std::fs::remove_file(&a).unwrap();
        write_record(&dir, t0 + 100, std::process::id(), Some("t1"), &[]);
        let text = diagnose(local.path(), "s-1", "hands", &run, Some("t1"), t0 + 31_000);
        assert!(text.contains("a process with its pid is still running") && text.contains("ownership unconfirmed"), "{text}");
        write_record(&dir, t0 + 200, i32::MAX as u32, Some("t1"), &[]);
        let text = diagnose(local.path(), "s-1", "hands", &run, Some("t1"), t0 + 31_000);
        assert!(text.contains("2 bot-hq hook records overlap") && text.contains("its process is gone") && text.contains("still running"), "{text}");
    }

    /// No record in the window: the evidence is missing — the hook may not
    /// have started, or wrote nothing, or already cleaned up — never a
    /// diagnosis of the shell. A record from another turn, or one that
    /// started after the run ended, does not count.
    #[test]
    fn no_overlapping_record_is_missing_evidence_not_a_diagnosis() {
        let local = local();
        let dir = dir_under(local.path(), "s-1", "hands").unwrap();
        let t0 = 1_761_500_000_000_i64;
        write_record(&dir, t0 + 500, 7, Some("t0"), &[("read-payload", t0 + 501)]);
        write_record(&dir, t0 + 40_000, 8, None, &[("read-payload", t0 + 40_001)]);
        let run = json!({"startedAt": t0, "durationMs": 30_000});
        let text = diagnose_with(local.path(), "s-1", "hands", &run, Some("t1"), t0 + 45_000, gone);
        assert!(text.contains("no bot-hq hook record overlaps that run"), "{text}");
        assert!(text.contains("may not have started in that window, or its record was never written or already removed"), "{text}");
        assert!(text.contains("the evidence is missing, so the stalled phase is unknown"), "{text}");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2, "other turns' records are left alone");
        // Nowhere to look at all (a data dir without the directory).
        let text = diagnose_with(local.path(), "s-9", "eyes", &run, None, t0 + 45_000, gone);
        assert!(text.contains("stalled phase is unknown"), "{text}");
        // A participant whose names are not path segments.
        let text = diagnose_with(local.path(), "../s", "hands", &run, None, t0 + 45_000, gone);
        assert!(text.contains("unknown"), "{text}");
    }

    /// Two records of the same turn overlapping the run (parallel tool calls)
    /// are listed and the owner is unknown, whatever their pids say; neither is
    /// deleted. An uncheckable process state is one more observation.
    #[test]
    fn overlapping_records_are_reported_not_chosen() {
        let local = local();
        let dir = dir_under(local.path(), "s-1", "hands").unwrap();
        let t0 = 1_761_500_000_000_i64;
        write_record(&dir, t0 + 100, 11, Some("t1"), &[("fetch-posture", t0 + 120)]);
        write_record(&dir, t0 + 900, 12, Some("t1"), &[("read-payload", t0 + 901)]);
        let run = json!({"startedAt": t0, "durationMs": 30_000});
        let text = diagnose_with(local.path(), "s-1", "hands", &run, Some("t1"), t0 + 31_000, gone);
        assert!(text.contains("2 bot-hq hook records overlap that run without a shared invocation id"), "{text}");
        assert!(text.contains("pid 11 started +100 ms, turn t1, last recorded phase fetch-posture, its process is gone; pid 12 started +900 ms, turn t1, last recorded phase read-payload, its process is gone"), "{text}");
        assert!(text.contains("which one Codex gave up on is unknown"), "{text}");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        let text = diagnose_with(local.path(), "s-1", "hands", &run, Some("t1"), t0 + 31_000, |pid| (pid == 12).then_some(false));
        assert!(text.contains("pid 11 started +100 ms, turn t1, last recorded phase fetch-posture, its process state could not be checked"), "{text}");
        assert!(text.contains("unknown") && !text.contains("most likely"), "{text}");
    }

    /// A record older than the run by more than the stale bound is a leftover
    /// nobody consumed, and is deleted by whoever looks next — the hook's
    /// `begin` and the host's `diagnose` alike; a record without a turn id
    /// (killed before the payload was read) still overlaps by time; a stamp's
    /// unit is read off its magnitude.
    #[test]
    fn stale_records_are_swept_and_unstamped_ones_still_match_by_time() {
        let local = local();
        let dir = dir_under(local.path(), "s-1", "hands").unwrap();
        let t0 = 1_761_500_000_000_i64;
        let stale = write_record(&dir, t0 - STALE_AFTER_MS - 10_000, 1, Some("old"), &[]);
        let fresh = write_record(&dir, t0 + 50, 2, None, &[]);
        let run = json!({"startedAt": t0 / 1000, "durationMs": 30_000});
        let text = diagnose_with(local.path(), "s-1", "hands", &run, Some("t1"), t0 + 31_000, gone);
        assert!(text.contains("one bot-hq hook record overlaps that run") && text.contains("pid 2 started +50 ms, no phase recorded, its process is gone (killed before reading the payload, or its writes failed)"), "{text}");
        assert!(!stale.exists(), "the stale leftover is swept");
        assert!(fresh.exists(), "the candidate stays until it goes stale");
        let stale = write_record(&dir, now_ms() - STALE_AFTER_MS - 10_000, 3, Some("old"), &[]);
        let rec = PhaseRecorder::begin(local.path(), "codex-gate", Some("s-1"), "hands");
        assert!(!stale.exists(), "begin sweeps too");
        rec.finish();
        // The stamp's unit is read off its magnitude.
        for (value, ms) in [(1_761_500_000_i64, t0), (t0, t0), (t0 * 1000, t0), (t0 * 1_000_000, t0)] {
            assert_eq!(epoch_ms(&json!(value)), Some(ms), "{value}");
        }
        assert_eq!(epoch_ms(&json!(0)), None);
        assert_eq!(epoch_ms(&json!("1761500000000")), None, "a string is not a stamp");
    }
}
