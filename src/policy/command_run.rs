//! Unix command ownership: keep the leader unreaped until the last group signal.
//! The HTTP future may disappear; only cleanup, not command execution, survives it.

use crate::agents::spawn::CHILD_PIDS;
use std::{io, process::Output, time::Duration};
use tokio::{io::AsyncReadExt, process::Child, sync::oneshot};

pub(crate) const INTERRUPT_GRACE: Duration = Duration::from_secs(2);
pub(crate) const TERMINATE_GRACE: Duration = Duration::from_secs(2);
pub(crate) const REAP_BOUND: Duration = Duration::from_secs(2);

struct OwnedChild {
    child: Child,
    pid: u32,
    armed: bool,
}

impl OwnedChild {
    fn new(child: Child) -> Self {
        let pid = child.id().expect("newly spawned child has a pid");
        CHILD_PIDS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(pid);
        Self {
            child,
            pid,
            armed: true,
        }
    }

    fn signal(&self, signal: i32) {
        if self.armed {
            // SAFETY: setsid made pid the group leader. This owner has not
            // reaped it, so the number cannot refer to an unrelated process.
            unsafe {
                libc::kill(-(self.pid as i32), signal);
            }
        }
    }

    fn try_reap(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
        // The exit reaper signals under this same lock. Unregister before
        // reaping; if the leader is still live, restore registration before
        // releasing the lock. No blocking wait while holding a std mutex.
        let mut pids = CHILD_PIDS.lock().unwrap_or_else(|e| e.into_inner());
        pids.remove(&self.pid);
        match self.child.try_wait() {
            Ok(Some(status)) => {
                self.armed = false;
                Ok(Some(status))
            }
            result => {
                if result
                    .as_ref()
                    .is_err_and(|e| e.raw_os_error() == Some(libc::ECHILD))
                {
                    // Someone else reaped it. Identity is no longer ours;
                    // never send a delayed signal to the old pid/group.
                    self.armed = false;
                } else {
                    pids.insert(self.pid);
                }
                result
            }
        }
    }

    async fn wait(&mut self) -> io::Result<std::process::ExitStatus> {
        loop {
            if let Some(status) = self.try_reap()? {
                return Ok(status);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn stop(&mut self) -> bool {
        if !self.armed {
            return true;
        }
        self.signal(libc::SIGINT);
        tokio::time::sleep(INTERRUPT_GRACE).await;
        self.signal(libc::SIGTERM);
        tokio::time::sleep(TERMINATE_GRACE).await;
        self.signal(libc::SIGKILL);
        matches!(
            tokio::time::timeout(REAP_BOUND, self.wait()).await,
            Ok(Ok(_))
        )
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.armed {
            // Runtime shutdown/panic fallback. Order matters: kill while
            // identity is reserved, unregister, THEN let tokio drop/reap Child.
            self.signal(libc::SIGKILL);
            CHILD_PIDS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&self.pid);
        }
    }
}

pub(super) async fn output(child: Child, timeout: Duration) -> io::Result<Output> {
    let mut owner = OwnedChild::new(child);
    let (cancel, mut cancelled) = oneshot::channel::<()>();
    let worker = tokio::spawn(async move {
        let mut stdout = owner.child.stdout.take().expect("piped stdout");
        let mut stderr = owner.child.stderr.take().expect("piped stderr");
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let result = tokio::select! {
            _ = &mut cancelled => Err(io::Error::new(io::ErrorKind::Interrupted, "command request cancelled")),
            result = tokio::time::timeout(timeout, async {
                tokio::try_join!(stdout.read_to_end(&mut out), stderr.read_to_end(&mut err))?;
                // Pipes first, then reap, all under the SAME execution bound.
                // A leader that exits while a grandchild owns a pipe stays ours.
                owner.wait().await
            }) => match result {
                Ok(result) => result,
                Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "command execution bound elapsed")),
            }
        };
        match result {
            Ok(status) => Ok(Output {
                status,
                stdout: out,
                stderr: err,
            }),
            Err(error) => {
                let confirmed = owner.stop().await;
                Err(io::Error::new(
                    error.kind(),
                    format!(
                        "{error}; {}",
                        if confirmed {
                            "command group signalled; leader reaped"
                        } else {
                            "command group signalled; leader cleanup unconfirmed"
                        }
                    ),
                ))
            }
        }
    });
    // Dropping the request closes this sender. The worker then stops the
    // group; it does not continue the command to its original deadline.
    let result = worker.await.map_err(io::Error::other)?;
    drop(cancel);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::tool_gate::run_in_repo;
    use std::path::Path;

    struct Probe {
        dir: tempfile::TempDir,
        run: tokio::task::JoinHandle<crate::policy::tool_gate::CommandOutput>,
    }

    impl Probe {
        async fn start(ignore: bool, leader_exits: bool, timeout: Duration) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let trap = if ignore {
                "trap '' INT TERM"
            } else {
                "trap 'echo INT > interrupted; exit 0' INT"
            };
            std::fs::write(dir.path().join("child.sh"), format!(
                "{trap}\necho $$ > child\nsleep 30 &\necho $! > grandchild\nwait\necho SHOULD_NOT_FINISH > finished\n")).unwrap();
            let path = dir.path().to_path_buf();
            let command = if leader_exits {
                "echo $$ > leader; sh child.sh & exit 0"
            } else {
                "echo $$ > leader; sh child.sh; :"
            };
            let run = tokio::spawn(async move { run_in_repo(command, &path, timeout, &[]).await });
            let probe = Self { dir, run };
            tokio::time::timeout(Duration::from_secs(5), async {
                while probe.pids().len() != 3 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("command must write its process identities");
            probe
        }

        fn pids(&self) -> Vec<u32> {
            pids(self.dir.path())
        }
        async fn gone(&self) -> bool {
            tokio::time::timeout(Duration::from_secs(7), async {
                while self.pids().iter().any(|pid| !crate::process::is_gone(*pid))
                    || CHILD_PIDS.lock().unwrap().contains(&self.pids()[0])
                {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            })
            .await
            .is_ok()
        }
    }

    fn pids(path: &Path) -> Vec<u32> {
        ["leader", "child", "grandchild"]
            .into_iter()
            .filter_map(|name| {
                std::fs::read_to_string(path.join(name))
                    .ok()?
                    .trim()
                    .parse()
                    .ok()
            })
            .collect()
    }

    impl Drop for Probe {
        fn drop(&mut self) {
            self.run.abort();
            for pid in self.pids() {
                if !crate::process::is_gone(pid) {
                    crate::process::kill_process_tree(pid);
                }
            }
        }
    }

    #[tokio::test]
    async fn command_group_has_its_own_session_and_inherited_group() {
        let probe = Probe::start(false, false, Duration::from_secs(10)).await;
        let ids = probe.pids();
        let leader = ids[0] as i32;
        // The runner may already lack a tty; identity is the real setsid proof.
        let identity = unsafe {
            (
                libc::getsid(leader),
                libc::getpgid(leader),
                libc::getpgid(ids[2] as i32),
            )
        };
        assert_eq!(identity, (leader, leader, leader));
    }

    #[tokio::test]
    async fn command_group_timeout_stops_even_uncooperative_descendants() {
        let mut probe = Probe::start(true, false, Duration::from_millis(750)).await;
        let out = (&mut probe.run).await.unwrap();
        let gone = probe.gone().await;
        assert_eq!(out.code, 124, "{}", out.stderr);
        assert!(gone, "descendant survived the command's bound");
        assert!(!probe.dir.path().join("finished").exists());
        assert!(!CHILD_PIDS.lock().unwrap().contains(&probe.pids()[0]));
    }

    #[tokio::test]
    async fn command_group_request_drop_interrupts_before_escalating() {
        let probe = Probe::start(false, false, Duration::from_secs(30)).await;
        probe.run.abort();
        assert!(
            probe.gone().await,
            "request cancellation must stop children"
        );
        assert_eq!(
            std::fs::read_to_string(probe.dir.path().join("interrupted")).unwrap(),
            "INT\n"
        );
        assert!(!CHILD_PIDS.lock().unwrap().contains(&probe.pids()[0]));
    }

    #[tokio::test]
    async fn command_group_keeps_the_exited_leader_until_pipe_cleanup() {
        let mut probe = Probe::start(true, true, Duration::from_millis(750)).await;
        let leader = probe.pids()[0];
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            CHILD_PIDS.lock().unwrap().contains(&leader),
            "leader must remain registered while pipes are open"
        );
        // Darwin getpgid(zombie) returns ESRCH. A non-reaping wait proves
        // ownership directly; the live grandchild proves the group's id.
        let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
        assert_eq!(
            unsafe {
                libc::waitid(
                    libc::P_PID,
                    leader as _,
                    info.as_mut_ptr(),
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            },
            0
        );
        assert_eq!(
            unsafe { libc::getpgid(probe.pids()[2] as i32) },
            leader as i32
        );
        assert_eq!((&mut probe.run).await.unwrap().code, 124);
        assert!(probe.gone().await);
    }

    #[tokio::test]
    async fn successful_command_unregisters_before_return() {
        let dir = tempfile::tempdir().unwrap();
        let out = run_in_repo("echo $$", dir.path(), Duration::from_secs(5), &[]).await;
        assert_eq!(out.code, 0, "{}", out.stderr);
        let pid = out.stdout.trim().parse().unwrap();
        assert!(!CHILD_PIDS.lock().unwrap().contains(&pid));
    }

    #[tokio::test]
    async fn last_resort_drop_kills_then_unregisters_before_child_drop() {
        use std::os::unix::process::CommandExt;
        let mut cmd = crate::process::tokio_command("sleep");
        cmd.arg("30").kill_on_drop(true);
        cmd.as_std_mut().process_group(0);
        let owner = OwnedChild::new(cmd.spawn().unwrap());
        let pid = owner.pid;
        assert!(CHILD_PIDS.lock().unwrap().contains(&pid));
        drop(owner);
        assert!(!CHILD_PIDS.lock().unwrap().contains(&pid));
        tokio::time::timeout(Duration::from_secs(2), async {
            while !crate::process::is_gone(pid) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    }

    // Feedback #129's cause, run only in the disposable subprocess below: the
    // test process itself starts with SIGINT IGNORED (as a `cargo test &` or
    // a `"$BIN" | tee &` launcher leaves its children), and the probe must
    // still see the graceful interrupt — which it cannot unless the spawner
    // resets the disposition, because a POSIX shell cannot trap a signal that
    // was ignored on entry.
    #[test]
    #[ignore = "private subprocess fixture: the launcher ignores SIGINT"]
    fn inherited_sigint_fixture() {
        let path = std::path::PathBuf::from(std::env::var_os("BOT_HQ_GATE_SIGINT_FIXTURE").unwrap());
        // The premise, checked rather than assumed: our own SIGINT is ignored.
        let ignored = unsafe {
            let mut current: libc::sigaction = std::mem::zeroed();
            libc::sigaction(libc::SIGINT, std::ptr::null(), &mut current);
            current.sa_sigaction == libc::SIG_IGN
        };
        assert!(ignored, "the fixture must be started with SIGINT ignored");
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            std::fs::write(
                path.join("child.sh"),
                "trap 'echo INT > interrupted; exit 0' INT\necho $$ > child\nsleep 30 &\necho $! > grandchild\nwait\necho SHOULD_NOT_FINISH > finished\n",
            )
            .unwrap();
            let cwd = path.clone();
            let run = tokio::spawn(async move {
                run_in_repo("echo $$ > leader; sh child.sh; :", &cwd, Duration::from_secs(30), &[]).await
            });
            tokio::time::timeout(Duration::from_secs(5), async {
                while pids(&path).len() != 3 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("command must write its process identities");
            run.abort();
            let ids = pids(&path);
            let gone = tokio::time::timeout(Duration::from_secs(7), async {
                while ids.iter().any(|pid| !crate::process::is_gone(*pid)) {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            })
            .await
            .is_ok();
            assert!(gone, "request cancellation must stop children");
            assert_eq!(
                std::fs::read_to_string(path.join("interrupted")).unwrap_or_default(),
                "INT\n",
                "the probe's shell must have trapped the graceful SIGINT although its launcher ignored the signal"
            );
        });
    }

    #[tokio::test]
    async fn a_launcher_that_ignores_sigint_still_lets_a_gated_command_trap_it() {
        use std::os::unix::process::CommandExt;
        let dir = tempfile::tempdir().unwrap();
        let mut cmd = crate::process::tokio_command(std::env::current_exe().unwrap());
        cmd.args([
            "--exact",
            "policy::command_run::tests::inherited_sigint_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env("BOT_HQ_GATE_SIGINT_FIXTURE", dir.path())
        .kill_on_drop(true);
        // SAFETY: signal(2) is async-signal-safe; nothing else runs in the
        // pre-exec closure.
        unsafe {
            cmd.as_std_mut().pre_exec(|| {
                libc::signal(libc::SIGINT, libc::SIG_IGN);
                Ok(())
            });
        }
        let out = tokio::time::timeout(Duration::from_secs(20), cmd.output()).await.unwrap().unwrap();
        for &pid in &pids(dir.path()) {
            if !crate::process::is_gone(pid) {
                crate::process::kill_process_tree(pid);
            }
        }
        assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert_eq!(std::fs::read_to_string(dir.path().join("interrupted")).unwrap(), "INT\n");
    }

    // Run only in the disposable subprocess below: the global reaper must
    // never kill unrelated children in the shared cargo-test process.
    #[test]
    #[ignore = "private subprocess fixture for the signal-exit reaper"]
    fn app_exit_reaper_fixture() {
        let path = std::path::PathBuf::from(std::env::var_os("BOT_HQ_GATE_EXIT_FIXTURE").unwrap());
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let cwd = path.clone();
            tokio::spawn(async move {
                run_in_repo(
                    "echo $$ > leader; sh child.sh; :",
                    &cwd,
                    Duration::from_secs(30),
                    &[],
                )
                .await
            });
            tokio::time::timeout(Duration::from_secs(5), async {
                while pids(&path).len() != 3 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            assert!(CHILD_PIDS.lock().unwrap().contains(&pids(&path)[0]));
            // The signal/panic reaper must recover poisoned ownership state.
            let _=std::panic::catch_unwind(|| {
                let _guard=CHILD_PIDS.lock().unwrap();
                panic!("fixture poisons the isolated child registry");
            });
            crate::agents::spawn::reap_all_children();
            // Deliberately skip all Rust Drops, as main's signal path does.
            std::process::exit(0);
        });
    }

    #[tokio::test]
    async fn exit_reaper_reaches_host_commands_without_drop() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("child.sh"),
            "echo $$ > child\nsleep 30 &\necho $! > grandchild\nwait\n",
        )
        .unwrap();
        let mut cmd = crate::process::tokio_command(std::env::current_exe().unwrap());
        cmd.args([
            "--exact",
            "policy::command_run::tests::app_exit_reaper_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env("BOT_HQ_GATE_EXIT_FIXTURE", dir.path())
        .kill_on_drop(true);
        let result = tokio::time::timeout(Duration::from_secs(10), cmd.output()).await;
        let ids = pids(dir.path());
        let gone = tokio::time::timeout(Duration::from_secs(2), async {
            while ids.iter().any(|pid| !crate::process::is_gone(*pid)) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .is_ok();
        for &pid in &ids {
            if !crate::process::is_gone(pid) {
                crate::process::kill_process_tree(pid);
            }
        }
        let out = result.unwrap().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(ids.len(), 3);
        assert!(gone, "exit reaper left a host command alive: {ids:?}");
    }
}
