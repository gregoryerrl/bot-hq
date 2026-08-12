//! Credential-shaped content, refused before it leaves the machine (rc3 **P6**).
//!
//! The Context Library has a private remote and bot-hq now pushes to it. That
//! push is the moment a mistake becomes irreversible, so it is the moment worth
//! checking: **a production database credential file sat committed in that repo
//! for 153 commits** and was caught only because someone looked before the
//! first push. `.gitignore` stops accidents; it does not stop an agent running
//! `git add -f`, and it does not stop a key pasted into a markdown note.
//!
//! # What this refuses, and what it deliberately does not
//!
//! Two axes, both chosen for a low false-positive rate, because a scanner that
//! cries wolf gets bypassed and then protects nothing:
//!
//! * **Filename class** — `.env`, private keys, keystores. A TRACKED file with
//!   one of these names is already past `.gitignore`, which is exactly the
//!   `git add -f` case.
//! * **Content shape** — only *self-identifying* secret formats: PEM private
//!   key headers and vendor-prefixed tokens (`sk-ant-`, `ghp_`, `AKIA…`,
//!   `xoxb-`, …). These cannot be confused with prose.
//!
//! **No generic `password=` / `secret:` matching.** The Context Library is full
//! of prose *about* credentials — the incident above is written up in it, by
//! name — so a generic matcher would refuse every push forever on the strength
//! of a sentence describing why the scanner exists.

use std::path::Path;

/// A file that must not be pushed, and the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretHit {
    /// Path as the caller supplied it (repo-relative for a git scan).
    pub path: String,
    /// Human-readable cause, e.g. "an AWS access key id" — quoted into the
    /// refusal so the user can find the thing without a second tool.
    pub reason: &'static str,
}

/// Filename classes that are credential-bearing by convention. Matched on the
/// file NAME (case-insensitively), not the full path.
fn filename_reason(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    // `.env.example` / `.env.template` are the documented, value-less forms and
    // are explicitly allowed — the same carve-out the library's .gitignore makes.
    let example = lower.ends_with(".example") || lower.ends_with(".template");
    if (lower == ".env" || lower.ends_with(".env")) && !example {
        return Some("a .env file");
    }
    if lower.ends_with(".pem") || lower.ends_with(".key") {
        return Some("a private key or certificate file");
    }
    if lower.ends_with(".p12") || lower.ends_with(".pfx") || lower.ends_with(".keystore") {
        return Some("a keystore file");
    }
    if lower.starts_with("id_rsa") || lower.starts_with("id_ed25519") {
        return Some("an SSH private key");
    }
    if lower == "credentials" || lower == ".netrc" || lower == ".npmrc" || lower == ".pypirc" {
        return Some("a credentials file");
    }
    None
}

/// Self-identifying secret formats. Each pattern is a literal prefix plus a
/// shape check, which is cheap and keeps the dependency list unchanged (no
/// regex crate for four patterns).
fn content_reason(body: &str) -> Option<&'static str> {
    if body.contains("-----BEGIN") && body.contains("PRIVATE KEY-----") {
        return Some("a PEM private key block");
    }
    // AWS access key ids: `AKIA` + 16 uppercase alphanumerics.
    if has_prefixed_token(body, "AKIA", 16, |c| c.is_ascii_uppercase() || c.is_ascii_digit()) {
        return Some("an AWS access key id");
    }
    // GitHub tokens — classic (`ghp_`/`gho_`/`ghs_`/`ghu_`) and fine-grained.
    for prefix in ["ghp_", "gho_", "ghs_", "ghu_", "github_pat_"] {
        if has_prefixed_token(body, prefix, 20, |c| c.is_ascii_alphanumeric() || c == '_') {
            return Some("a GitHub access token");
        }
    }
    // Anthropic / OpenAI-style keys.
    for prefix in ["sk-ant-", "sk-proj-"] {
        if has_prefixed_token(body, prefix, 20, |c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Some("an API key");
        }
    }
    // Slack bot/user tokens.
    for prefix in ["xoxb-", "xoxp-", "xoxa-", "xoxs-"] {
        if has_prefixed_token(body, prefix, 10, |c| c.is_ascii_alphanumeric() || c == '-') {
            return Some("a Slack token");
        }
    }
    None
}

/// Does `body` contain `prefix` followed by at least `min_len` characters the
/// predicate accepts? The length floor is what keeps a *mention* of a prefix in
/// prose ("tokens starting with `ghp_`") from reading as a token.
fn has_prefixed_token(
    body: &str,
    prefix: &str,
    min_len: usize,
    accept: impl Fn(char) -> bool + Copy,
) -> bool {
    body.match_indices(prefix).any(|(i, _)| {
        body[i + prefix.len()..]
            .chars()
            .take_while(|c| accept(*c))
            .count()
            >= min_len
    })
}

/// Scan one file. `rel_path` is what a refusal will name; `body` is its
/// content, already read.
///
/// Binary or unreadable files are the caller's problem — this takes a `&str`
/// so "could not decode" is decided once, at the read.
pub fn scan_file(rel_path: &str, body: &str) -> Option<SecretHit> {
    let name = Path::new(rel_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(rel_path);
    let reason = filename_reason(name).or_else(|| content_reason(body))?;
    Some(SecretHit {
        path: rel_path.to_string(),
        reason,
    })
}

/// Render hits as the sentence a refusal shows. Names the FILES, because
/// "a secret was found" without a path leaves the user grepping.
pub fn refusal_message(hits: &[SecretHit]) -> String {
    let listed: Vec<String> = hits
        .iter()
        .map(|h| format!("{} ({})", h.path, h.reason))
        .collect();
    format!(
        "refusing to push: {} credential-shaped file(s) are tracked — {}. \
         Remove them from the repo (and rotate anything already exposed) before pushing.",
        hits.len(),
        listed.join("; ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_shaped_files_are_caught_by_name_or_by_content() {
        // By name — the `git add -f` case .gitignore cannot stop.
        assert_eq!(
            scan_file("projects/acme/prod.env", "DB_HOST=x").map(|h| h.reason),
            Some("a .env file")
        );
        assert_eq!(
            scan_file("deploy/id_rsa", "whatever").map(|h| h.reason),
            Some("an SSH private key")
        );
        // By content — a key pasted into an ordinary note.
        assert_eq!(
            scan_file(
                "notes.md",
                "here is the key:\n-----BEGIN RSA PRIVATE KEY-----\nMIIE…"
            )
            .map(|h| h.reason),
            Some("a PEM private key block")
        );
        assert_eq!(
            scan_file("notes.md", "AKIAIOSFODNN7EXAMPLE is the id").map(|h| h.reason),
            Some("an AWS access key id")
        );
        assert_eq!(
            scan_file("n.md", "token ghp_1234567890abcdefghijABCDEF").map(|h| h.reason),
            Some("a GitHub access token")
        );
    }

    /// The false-positive half, and it is the half that decides whether this
    /// scanner survives contact with the real library.
    ///
    /// The Context Library documents the very incident that motivated this
    /// check — by filename, and quoting token prefixes. A scanner that refuses
    /// every push because of a sentence gets turned off, and then it protects
    /// nothing.
    #[test]
    fn prose_about_secrets_is_not_a_secret() {
        assert_eq!(
            scan_file(
                "notes.md",
                "A production credential file (prod.env) sat committed for 153 commits. \
                 Tokens beginning `ghp_` or `sk-ant-` are refused, as are AKIA-prefixed ids. \
                 Set password=<yours> in the deploy form."
            ),
            None
        );
        // The documented, value-less template forms stay pushable.
        assert_eq!(scan_file("config/.env.example", "DB_HOST="), None);
        assert_eq!(scan_file("config/prod.env.template", "DB_HOST="), None);
    }

    #[test]
    fn the_refusal_names_every_file() {
        let msg = refusal_message(&[
            SecretHit {
                path: "projects/acme/prod.env".into(),
                reason: "a .env file",
            },
            SecretHit {
                path: "notes.md".into(),
                reason: "a PEM private key block",
            },
        ]);
        assert!(msg.contains("projects/acme/prod.env"));
        assert!(msg.contains("notes.md"));
        assert!(msg.contains("2 credential-shaped file(s)"));
    }
}
