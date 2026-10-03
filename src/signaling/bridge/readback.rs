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
//! request it adds the issues GitHub links it to close.
//!
//! Every read is a GET (`gh api` turns into a POST as soon as a field flag is
//! present — EYES, s-3158eb35), built only from parts of the URL gh printed
//! that passed a strict character check, and run through the same gate shell
//! as the publish, so the same rc files and PATH find the same `gh`.

use super::outward_body::{gh_publish, GhPublishKind, PublishedBody};
use super::*;
use crate::policy::tool_gate;

/// The bound on one read.
const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// How long a mismatched EDIT waits before its one re-read (EYES: GitHub may
/// serve the old body for a moment after an edit).
const EDIT_RETRY: std::time::Duration = std::time::Duration::from_secs(2);

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
}
