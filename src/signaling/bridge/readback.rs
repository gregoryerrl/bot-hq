//! Read an approved GitHub publish back (feedback #57 #60 #97).
//!
//! "Approved" and "landed as approved" are different facts. Reviewers checked
//! the second by hand after every publish — about 12 fetch-and-diff rounds in
//! one evening (#60), once writing the posted text to `/tmp` outside a
//! read-only role to diff it (#97) — and a naive check gave false mismatches
//! because `gh … --jq .body` drops the trailing newline (#57).
//!
//! After an approved single `gh issue|pr create|edit|comment` runs, bot-hq
//! reads the object back and compares it with the bytes that were approved,
//! and the gate's result row says which: equal, DIFFERS (with the first
//! differing line), could not be read, or not read back (with why). For a pull
//! request it adds the issues GitHub links it to close. For a queued EDIT the
//! reviewer's message also shows the diff against the live body, and the live
//! body's hash is re-checked before the approved edit runs.
//!
//! Every read is a GET (`gh api` turns into a POST as soon as a field flag is
//! present — EYES, s-3158eb35), built only from parts of the URL gh printed
//! that passed a strict character check, and run through the same gate shell
//! as the publish, so the same rc files and PATH find the same `gh`.

use super::outward_body::{gh_publish, GhPublish, GhPublishKind, PublishedBody};
use super::*;
use crate::policy::tool_gate;

/// The bound on one read.
const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// How long a mismatched EDIT waits before its one re-read (EYES: GitHub may
/// serve the old body for a moment after an edit).
const EDIT_RETRY: std::time::Duration = std::time::Duration::from_secs(2);
/// The diff in the reviewer's message stops here (EYES: a PR body can be
/// 64K characters).
const DIFF_MAX_LINES: usize = 200;
const DIFF_MAX_BYTES: usize = 16 * 1024;

/// A GitHub issue, pull request or comment, from a URL gh printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObjectRef {
    pub host: String,
    pub owner: String,
    pub repo: String,
    pub pull: bool,
    pub number: u64,
    pub comment: Option<u64>,
}

impl ObjectRef {
    /// The issue's or pull request's own URL.
    pub(crate) fn object_url(&self) -> String {
        let kind = if self.pull { "pull" } else { "issues" };
        format!("https://{}/{}/{}/{kind}/{}", self.host, self.owner, self.repo, self.number)
    }

    /// The URL as gh printed it, with the comment's fragment.
    pub(crate) fn url(&self) -> String {
        match self.comment {
            Some(id) => format!("{}#issuecomment-{id}", self.object_url()),
            None => self.object_url(),
        }
    }

    /// The REST path of what was published: the comment, or the issue's or
    /// pull request's own body (REST serves a pull request as an issue).
    fn api_path(&self) -> String {
        match self.comment {
            Some(id) => format!("repos/{}/{}/issues/comments/{id}", self.owner, self.repo),
            None => format!("repos/{}/{}/issues/{}", self.owner, self.repo, self.number),
        }
    }

    /// Whether this is the object `kind` publishes to — a comment URL for a
    /// comment, an issue URL for an issue, a pull request's for a pull request.
    fn fits(&self, kind: GhPublishKind) -> bool {
        match kind {
            GhPublishKind::IssueComment | GhPublishKind::PrComment => self.comment.is_some(),
            GhPublishKind::IssueCreate | GhPublishKind::IssueEdit => !self.pull && self.comment.is_none(),
            GhPublishKind::PrCreate | GhPublishKind::PrEdit => self.pull && self.comment.is_none(),
        }
    }
}

fn host_ok(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

fn name_ok(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn number(s: &str) -> Option<u64> {
    (!s.is_empty() && s.len() <= 19 && s.chars().all(|c| c.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
}

/// One URL gh printed, if it is exactly an issue, pull request or comment URL
/// whose every part passes the character check. Nothing that fails it ever
/// reaches a read command.
pub(crate) fn parse_object_url(line: &str) -> Option<ObjectRef> {
    let rest = line.trim().strip_prefix("https://")?;
    let (path, fragment) = match rest.split_once('#') {
        Some((p, f)) => (p, Some(f)),
        None => (rest, None),
    };
    let parts: Vec<&str> = path.split('/').collect();
    let [host, owner, repo, kind, num] = parts.as_slice() else {
        return None;
    };
    if !host_ok(host) || !name_ok(owner) || !name_ok(repo) {
        return None;
    }
    let pull = match *kind {
        "pull" => true,
        "issues" => false,
        _ => return None,
    };
    let comment = match fragment {
        None => None,
        Some(f) => Some(number(f.strip_prefix("issuecomment-")?)?),
    };
    Some(ObjectRef {
        host: host.to_string(),
        owner: owner.to_string(),
        repo: repo.to_string(),
        pull,
        number: number(num)?,
        comment,
    })
}

/// The object a publish names on stdout: its LAST line that is such a URL.
/// gh 2.81 prints only that URL on stdout for these commands (its "Creating
/// …" and warnings go to stderr — measured from its source, s-3158eb35).
pub(crate) fn object_from_stdout(stdout: &str) -> Option<ObjectRef> {
    stdout.lines().rev().find_map(parse_object_url)
}

/// Single-quote a word for a POSIX shell. The `gh` program can be any path
/// (a test's temp dir), so it is quoted rather than checked (EYES).
pub(crate) fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The read of what was published: a REST GET, said explicitly, built only
/// from checked parts.
pub(crate) fn read_command(gh: &str, obj: &ObjectRef) -> String {
    let host = if obj.host == "github.com" { String::new() } else { format!(" --hostname {}", obj.host) };
    format!("{} api --method GET{host} {}", sh_quote(gh), obj.api_path())
}

/// A pull request's closing references (#97).
pub(crate) fn closing_refs_command(gh: &str, obj: &ObjectRef) -> String {
    format!("{} pr view {} --json closingIssuesReferences", sh_quote(gh), obj.object_url())
}

/// The `body` of a REST issue, pull request or comment; a null body is "".
pub(crate) fn body_of(json: &str) -> Result<String, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("the reply was not JSON ({e})"))?;
    match v.get("body") {
        Some(serde_json::Value::String(s)) => Ok(s.clone()),
        Some(serde_json::Value::Null) => Ok(String::new()),
        _ => Err("the reply carried no body".to_string()),
    }
}

/// The issues a pull request closes, as `#12`, or `owner/repo#12` when they
/// live in another repository.
pub(crate) fn closing_refs_of(json: &str, obj: &ObjectRef) -> Result<Vec<String>, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("the reply was not JSON ({e})"))?;
    let list = v
        .get("closingIssuesReferences")
        .and_then(|l| l.as_array())
        .ok_or_else(|| "the reply carried no closingIssuesReferences".to_string())?;
    Ok(list
        .iter()
        .filter_map(|issue| {
            let n = issue.get("number")?.as_u64()?;
            let owner = issue.pointer("/repository/owner/login").and_then(|x| x.as_str());
            let name = issue.pointer("/repository/name").and_then(|x| x.as_str());
            Some(match (owner, name) {
                (Some(o), Some(r)) if !(o.eq_ignore_ascii_case(&obj.owner) && r.eq_ignore_ascii_case(&obj.repo)) => {
                    format!("{o}/{r}#{n}")
                }
                _ => format!("#{n}"),
            })
        })
        .collect())
}

/// How a published body compares with the approved one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Compared {
    Equal,
    /// The first line that differs, 1-based, as each side has it.
    Differs { line: usize, approved: String, published: String },
}

/// CRLF → LF, and only the END of the whole body trimmed: the published text
/// can lose its final newline (#57), but two spaces at the end of a line
/// INSIDE the body are a Markdown line break, so a body that lost them must
/// still read as different (EYES).
fn normalize(s: &str) -> String {
    s.replace("\r\n", "\n").trim_end().to_string()
}

pub(crate) fn compare(approved: &str, published: &str) -> Compared {
    let (a, p) = (normalize(approved), normalize(published));
    if a == p {
        return Compared::Equal;
    }
    let (al, pl): (Vec<&str>, Vec<&str>) = (a.split('\n').collect(), p.split('\n').collect());
    let i = (0..al.len().max(pl.len()))
        .find(|&i| al.get(i) != pl.get(i))
        .unwrap_or(0);
    let side = |lines: &[&str], what: &str| {
        lines.get(i).map(|l| clip(l, 120)).unwrap_or_else(|| format!("(the {what} body ends before it)"))
    };
    Compared::Differs { line: i + 1, approved: side(&al, "approved"), published: side(&pl, "published") }
}

fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// The view of an edit's TARGET, for its live body: `<gh> issue view <n>
/// --json body,url [-R owner/repo]`. `None` when the target or the repo is
/// not something the character check passes — a branch name, an odd repo.
pub(crate) fn live_view_command(gh: &str, publish: &GhPublish) -> Option<String> {
    let noun = match publish.kind {
        GhPublishKind::IssueEdit => "issue",
        GhPublishKind::PrEdit => "pr",
        _ => return None,
    };
    let target = publish.target.as_deref()?;
    let target = match (number(target), parse_object_url(target)) {
        (Some(n), _) => n.to_string(),
        (None, Some(obj)) if obj.comment.is_none() && obj.pull == (noun == "pr") => obj.object_url(),
        _ => return None,
    };
    let repo = match publish.repo.as_deref() {
        None => String::new(),
        Some(r) => {
            let parts: Vec<&str> = r.split('/').collect();
            let ok = match parts.as_slice() {
                [owner, name] => name_ok(owner) && name_ok(name),
                [host, owner, name] => host_ok(host) && name_ok(owner) && name_ok(name),
                _ => false,
            };
            if !ok {
                return None;
            }
            format!(" -R {r}")
        }
    };
    Some(format!("{} {noun} view {target} --json body,url{repo}", sh_quote(gh)))
}

/// `sha256` of a live body as read — the queue-time value and the
/// approval-time value are compared byte for byte.
fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(s.as_bytes()))
}

/// The unified diff of `old` → `new` from its first hunk, through `git diff
/// --no-index` (exact, and git is a prerequisite), capped for the message.
/// `Ok(None)` when they are the same.
fn unified_diff(old: &str, new: &str) -> Result<Option<String>, String> {
    unified_diff_with(old, new, &[])
}

/// [`unified_diff`] with extra environment for git — the seam a test uses to
/// configure an external diff tool without touching this process's env.
///
/// `--no-ext-diff --no-textconv`: a user's `diff.external` (difftastic is a
/// common one) or `GIT_EXTERNAL_DIFF` makes `git diff --no-index` print
/// NOTHING and still exit 1, which read as an empty change under "what this
/// edit changes" (EYES, advisory `268a8c8a`, measured on git 2.50.1).
fn unified_diff_with(old: &str, new: &str, envs: &[(&str, &str)]) -> Result<Option<String>, String> {
    let dir = tempfile::tempdir().map_err(|e| format!("a temp dir: {e}"))?;
    let (a, b) = (dir.path().join("live"), dir.path().join("approved"));
    std::fs::write(&a, old).map_err(|e| format!("writing the live body: {e}"))?;
    std::fs::write(&b, new).map_err(|e| format!("writing the approved body: {e}"))?;
    let out = std::process::Command::new("git")
        .args(["diff", "--no-index", "--no-color", "--no-ext-diff", "--no-textconv", "-U3", "--"])
        .arg(&a)
        .arg(&b)
        .envs(envs.iter().copied())
        .output()
        .map_err(|e| format!("git diff: {e}"))?;
    diff_from_git(
        out.status.code(),
        &String::from_utf8_lossy(&out.stdout),
        &String::from_utf8_lossy(&out.stderr),
    )
}

/// What `git diff --no-index` said: exit 0 is "the same", exit 1 is a diff —
/// shown from its first hunk and capped — and exit 1 with NO hunk is an
/// error, never an empty change (EYES, `268a8c8a`).
fn diff_from_git(code: Option<i32>, stdout: &str, stderr: &str) -> Result<Option<String>, String> {
    match code {
        Some(0) => Ok(None),
        Some(1) => {
            let hunks: Vec<&str> = stdout.lines().skip_while(|l| !l.starts_with("@@")).collect();
            if hunks.is_empty() {
                return Err("git diff reported a difference but printed no hunk".to_string());
            }
            let mut shown = String::new();
            for (n, line) in hunks.iter().enumerate() {
                if n >= DIFF_MAX_LINES || shown.len() + line.len() > DIFF_MAX_BYTES {
                    shown.push_str(&format!("… {} more diff line(s) not shown\n", hunks.len() - n));
                    break;
                }
                shown.push_str(line);
                shown.push('\n');
            }
            Ok(Some(shown))
        }
        _ => Err(format!("git diff: {}", stderr.trim())),
    }
}

impl SignalingBridge {
    /// The `gh` the reads run: the gate shell's own `gh`, or the program a
    /// test set with [`Self::set_gh_program`].
    fn gh_program(&self) -> String {
        self.gh_program
            .get()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "gh".to_string())
    }

    /// Point the publish reads at another `gh` — the test seam (a fake that
    /// prints what gh 2.81 prints). Set once; later calls are ignored.
    pub fn set_gh_program(&self, program: PathBuf) {
        let _ = self.gh_program.set(program);
    }

    /// Run one read through the gate shell, in the session's repo, with its
    /// env — the shell, rc files and PATH the publish itself ran with (EYES:
    /// an app started from the Dock may not have Homebrew's `gh` on a bare
    /// PATH). Its stdout, or the first line of what went wrong.
    async fn gh_read(&self, session_id: &str, command: &str) -> Result<String, String> {
        let cwd = self
            .session_working_repo(session_id)
            .await
            .ok_or_else(|| "the session has no working repo".to_string())?;
        let envs = tool_gate::session_envs(session_id);
        let envs: Vec<(&str, &str)> = envs.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let out = tool_gate::run_in_repo(command, &cwd, READ_TIMEOUT, &envs).await;
        if out.code == 0 {
            return Ok(out.stdout);
        }
        let why = out
            .stderr
            .lines()
            .chain(out.stdout.lines())
            .map(str::trim)
            .find(|l| !l.is_empty())
            .map(|l| clip(l, 160))
            .unwrap_or_else(|| format!("exit {}", out.code));
        Err(why)
    }

    async fn fetch_and_compare(&self, session_id: &str, read: &str, approved: &str) -> Result<Compared, String> {
        let json = self.gh_read(session_id, read).await?;
        let published = body_of(&json)?;
        Ok(compare(approved, &published))
    }

    /// The block an approved outward publish's result row gains once it ran
    /// (feedback #57 #60 #97). `stdout` is the publish's own; `approved_files`
    /// are the bytes the approval's hash check read — never a later re-read,
    /// since agents reuse their `/tmp` draft paths (EYES).
    pub(super) async fn read_back_publish(
        &self,
        session_id: &str,
        command: &str,
        stdout: &str,
        exit_code: i32,
        approved_files: &[(String, Vec<u8>)],
    ) -> String {
        let publish = match gh_publish(command) {
            Ok(p) => p,
            Err(why) => return format!("\nNot read back: {why}.\n"),
        };
        let Some(obj) = object_from_stdout(stdout) else {
            return if exit_code == 0 {
                "\nNot read back: the publish printed no issue, pull request or comment URL, so \
                 bot-hq cannot tell which object it wrote — check it on GitHub.\n"
                    .to_string()
            } else {
                // EYES: `gh pr create` creates the PR, then adds labels and
                // reviewers; a failure in the second step exits non-zero after
                // something was published.
                "\nNot read back: the command failed and printed no URL — but gh may still have \
                 published part of it. Check on GitHub before retrying, or a retry may publish a \
                 duplicate.\n"
                    .to_string()
            };
        };
        if !obj.fits(publish.kind) {
            return format!(
                "\nNot read back: the URL gh printed ({}) is not the kind of object this command \
                 publishes to — compare it by hand.\n",
                obj.url()
            );
        }
        let (approved, source) = match &publish.body {
            PublishedBody::Inline(text) => (text.clone(), "the inline body".to_string()),
            PublishedBody::File(path) => match approved_files.iter().find(|(p, _)| p == path) {
                Some((_, bytes)) => (String::from_utf8_lossy(bytes).into_owned(), format!("body file `{path}`")),
                None => {
                    return format!(
                        "\nNot read back: the approved content of `{path}` was not kept at \
                         approval — compare it by hand: {}\n",
                        obj.url()
                    )
                }
            },
        };
        let gh = self.gh_program();
        let read = read_command(&gh, &obj);
        let shown = read.replacen(&sh_quote(&gh), "gh", 1);
        let mut outcome = self.fetch_and_compare(session_id, &read, &approved).await;
        if matches!(outcome, Ok(Compared::Differs { .. })) && publish.kind.replaces(publish.edit_last) {
            tokio::time::sleep(EDIT_RETRY).await;
            outcome = self.fetch_and_compare(session_id, &read, &approved).await;
        }
        let mut out = String::new();
        match outcome {
            Ok(Compared::Equal) => out.push_str(&format!(
                "\nRead back (`{shown}`): the published body equals what was approved ({source}, \
                 {} bytes): {}\n",
                approved.len(),
                obj.url()
            )),
            Ok(Compared::Differs { line, approved: a, published: p }) => out.push_str(&format!(
                "\nRead back (`{shown}`): ⚠ the published body DIFFERS from what was approved \
                 ({source}) — first at line {line}: approved {a:?} / published {p:?}. Check it: {}\n",
                obj.url()
            )),
            Err(why) => out.push_str(&format!(
                "\nRead back: could not read it back ({why}) — compare it by hand: {}\n",
                obj.url()
            )),
        }
        if exit_code != 0 {
            out.push_str(
                "The command exited non-zero after publishing: gh may not have finished every \
                 step (labels, reviewers). Check before retrying, or a retry may publish a \
                 duplicate.\n",
            );
        }
        if publish.kind.is_pr_body() {
            let refs = self
                .gh_read(session_id, &closing_refs_command(&gh, &obj))
                .await
                .and_then(|json| closing_refs_of(&json, &obj));
            match refs {
                Ok(refs) if refs.is_empty() => out.push_str("GitHub links it to close no issue.\n"),
                Ok(refs) => out.push_str(&format!("GitHub links it to close: {}.\n", refs.join(", "))),
                Err(why) => out.push_str(&format!("Its closing references could not be read ({why}).\n")),
            }
        }
        out
    }

    /// An edit's target, read live: `(body, url)`. `Ok(None)` when `command`
    /// is not an issue or pull request edit whose target and repo pass the
    /// character check.
    async fn live_body(&self, session_id: &str, command: &str) -> Result<Option<(String, String)>, String> {
        let Ok(publish) = gh_publish(command) else {
            return Ok(None);
        };
        let Some(view) = live_view_command(&self.gh_program(), &publish) else {
            return Ok(None);
        };
        let json = self.gh_read(session_id, &view).await?;
        let body = body_of(&json)?;
        let url = serde_json::from_str::<serde_json::Value>(&json)
            .ok()
            .and_then(|v| v.get("url").and_then(|u| u.as_str()).map(str::to_string))
            .unwrap_or_default();
        Ok(Some((body, url)))
    }

    /// For a queued EDIT (feedback #60's first point): the section the
    /// reviewer's message gains — the diff of the approved body against the
    /// live one — and the live body's hash, re-checked before the approved
    /// edit runs. `None` for anything that is not such an edit.
    pub(super) async fn live_edit_section(
        &self,
        session_id: &str,
        command: &str,
    ) -> Option<(String, Option<String>)> {
        let publish = gh_publish(command).ok()?;
        if !matches!(publish.kind, GhPublishKind::IssueEdit | GhPublishKind::PrEdit) {
            return None;
        }
        let approved = match &publish.body {
            PublishedBody::Inline(text) => text.clone(),
            PublishedBody::File(path) => {
                let resolved = self.resolve_body_path(session_id, path).await;
                std::fs::read_to_string(&resolved).ok()?
            }
        };
        match self.live_body(session_id, command).await {
            Ok(None) => Some((
                "--- (the live body was not read: the edit's target is not an issue or pull \
                 request number or URL bot-hq can check) ---"
                    .to_string(),
                None,
            )),
            Err(why) => Some((format!("--- (the live body could not be read: {why}) ---"), None)),
            Ok(Some((live, url))) => {
                let sha = Some(sha256_hex(&live));
                let section = match unified_diff(&live, &approved) {
                    Ok(None) => format!("--- (this edit changes nothing in the live body of {url}) ---"),
                    Ok(Some(diff)) => format!(
                        "--- what this edit changes against the live body of {url} (read at queue \
                         time) ---\n{}",
                        diff.trim_end()
                    ),
                    Err(why) => format!("--- (the diff against the live body of {url} failed: {why}) ---"),
                };
                Some((section, sha))
            }
        }
    }

    /// The live body's hash now, for the approval-time re-check of an edit
    /// whose queue-time hash was recorded.
    pub(super) async fn live_body_sha(&self, session_id: &str, command: &str) -> Result<Option<String>, String> {
        Ok(self.live_body(session_id, command).await?.map(|(body, _)| sha256_hex(&body)))
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(url: &str) -> ObjectRef {
        parse_object_url(url).unwrap_or_else(|| panic!("{url} parses"))
    }

    #[test]
    fn urls_gh_prints_parse_and_everything_else_does_not() {
        let c = obj("https://github.com/o/r/issues/5#issuecomment-77");
        assert_eq!((c.number, c.comment, c.pull), (5, Some(77), false));
        let p = obj("https://github.com/acme-co/web.app/pull/45");
        assert_eq!((p.owner.as_str(), p.repo.as_str(), p.pull, p.comment), ("acme-co", "web.app", true, None));
        assert_eq!(obj("https://ghe.example.com/o/r/issues/3").host, "ghe.example.com");
        for junk in [
            "http://github.com/o/r/issues/5",
            "https://github.com/o/r/issues/5;rm",
            "https://github.com/o/r/issues/five",
            "https://github.com/o/r/discussions/5",
            "https://github.com/o/r/issues/5#comment-1",
            "https://github.com/o/../issues/5",
            "https://github.com/o r/x/issues/5",
            "https://github.com/$(id)/r/issues/5",
            "https://github.com/o/r/issues/5/extra",
            "Creating pull request for me:topic into main in o/r",
        ] {
            assert_eq!(parse_object_url(junk), None, "{junk}");
        }
        // The LAST URL line of stdout.
        let out = "https://github.com/o/r/issues/1\nhttps://github.com/o/r/issues/2\n\n";
        assert_eq!(object_from_stdout(out).map(|o| o.number), Some(2));
        assert_eq!(object_from_stdout("nothing here\n"), None);
    }

    /// EYES (s-3158eb35): the read is a GET, said explicitly, built from
    /// checked parts, with the program quoted — pinned exactly.
    #[test]
    fn the_read_commands_are_pinned() {
        let c = obj("https://github.com/o/r/issues/5#issuecomment-77");
        assert_eq!(read_command("gh", &c), "'gh' api --method GET repos/o/r/issues/comments/77");
        let p = obj("https://github.com/o/r/pull/45");
        assert_eq!(read_command("gh", &p), "'gh' api --method GET repos/o/r/issues/45");
        assert_eq!(
            read_command("/tmp/it's here/gh", &p),
            "'/tmp/it'\\''s here/gh' api --method GET repos/o/r/issues/45"
        );
        let e = obj("https://ghe.example.com/o/r/issues/3");
        assert_eq!(read_command("gh", &e), "'gh' api --method GET --hostname ghe.example.com repos/o/r/issues/3");
        assert_eq!(
            closing_refs_command("gh", &p),
            "'gh' pr view https://github.com/o/r/pull/45 --json closingIssuesReferences"
        );
    }

    #[test]
    fn the_live_view_runs_only_on_a_checked_edit_target() {
        let view = |c: &str| live_view_command("gh", &gh_publish(c).unwrap());
        assert_eq!(view("gh issue edit 63 --body-file b.md").as_deref(), Some("'gh' issue view 63 --json body,url"));
        assert_eq!(
            view("gh pr edit https://github.com/o/r/pull/9 -b x -R o/r").as_deref(),
            Some("'gh' pr view https://github.com/o/r/pull/9 --json body,url -R o/r")
        );
        assert_eq!(view("gh pr edit my-branch -b x"), None, "a branch is not checked");
        assert_eq!(view("gh issue edit 5 -b x -R 'o/r;x'"), None, "an odd repo");
        assert_eq!(view("gh issue edit https://github.com/o/r/pull/9 -b x"), None, "a PR URL on issue edit");
        assert_eq!(view("gh issue comment 5 -b x"), None, "not an edit");
    }

    #[test]
    fn bodies_and_closing_references_read_from_the_raw_json() {
        assert_eq!(body_of(r#"{"body":"line\n","id":1}"#).unwrap(), "line\n");
        assert_eq!(body_of(r#"{"body":null}"#).unwrap(), "");
        assert!(body_of("{}").is_err() && body_of("not json").is_err());
        let p = obj("https://github.com/o/r/pull/45");
        let refs = closing_refs_of(
            r#"{"closingIssuesReferences":[
                {"number":12,"repository":{"name":"r","owner":{"login":"o"}}},
                {"number":7,"repository":{"name":"other","owner":{"login":"o"}}}]}"#,
            &p,
        )
        .unwrap();
        assert_eq!(refs, vec!["#12", "o/other#7"]);
        assert!(closing_refs_of(r#"{"closingIssuesReferences":[]}"#, &p).unwrap().is_empty());
    }

    /// Only the END of the whole body is trimmed (#57's lost final newline,
    /// CRLF); a line's trailing double space — a Markdown line break — still
    /// counts (EYES).
    #[test]
    fn the_comparison_trims_only_the_end_of_the_body() {
        assert_eq!(compare("a\nb\n", "a\nb"), Compared::Equal);
        assert_eq!(compare("a\r\nb\r\n", "a\nb\n\n"), Compared::Equal);
        assert_eq!(
            compare("one  \ntwo\n", "one\ntwo\n"),
            Compared::Differs { line: 1, approved: "one  ".into(), published: "one".into() }
        );
        assert_eq!(
            compare("a\nb\nc\n", "a\nb\n"),
            Compared::Differs { line: 3, approved: "c".into(), published: "(the published body ends before it)".into() }
        );
    }

    #[test]
    fn a_diff_is_capped_for_the_message() {
        let old: String = (0..1000).map(|i| format!("old {i}\n")).collect();
        let new: String = (0..1000).map(|i| format!("new {i}\n")).collect();
        let diff = unified_diff(&old, &new).unwrap().expect("they differ");
        assert!(diff.starts_with("@@"), "{}", &diff[..40]);
        assert!(diff.lines().count() <= DIFF_MAX_LINES + 1 && diff.len() <= DIFF_MAX_BYTES + 200);
        assert!(diff.contains("more diff line(s) not shown"));
        assert_eq!(unified_diff("same\n", "same\n").unwrap(), None);
    }

    /// EYES, advisory `268a8c8a`: an external diff tool configured for git
    /// (`GIT_EXTERNAL_DIFF`, `diff.external`) made the diff print nothing and
    /// exit 1. The diff ignores it, and "exit 1 with no hunk" is an error,
    /// never an empty change.
    #[test]
    fn the_diff_ignores_an_external_diff_tool_and_never_reads_empty_as_a_change() {
        let diff = unified_diff_with("old\n", "new\n", &[("GIT_EXTERNAL_DIFF", "true")])
            .unwrap()
            .expect("they differ");
        assert!(diff.contains("-old") && diff.contains("+new"), "{diff}");
        assert!(diff_from_git(Some(1), "", "").is_err());
        assert!(diff_from_git(Some(1), "diff --git a/x b/x\n", "").is_err());
        assert_eq!(diff_from_git(Some(0), "", "").unwrap(), None);
    }
}
