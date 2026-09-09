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
pub(crate) fn filename_reason(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    // `.env.example` / `.env.template` are the documented, value-less forms and
    // are explicitly allowed — the same carve-out the library's .gitignore makes.
    let example = lower.ends_with(".example") || lower.ends_with(".template");
    // `.env`, `.env.local`, `.env.production`, `.envrc` (direnv), `prod.env`:
    // the prefix form is the common secret-bearing shape in JS projects and
    // was missed until 1.0.5 (only `== ".env"` / `ends_with(".env")`).
    if (lower.starts_with(".env") || lower.ends_with(".env")) && !example {
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
    // The families the 2026-09-06 sweep found missing — each still a literal
    // vendor prefix plus a length floor, so a prose mention stays clean.
    // Bare `sk-` keys (DeepSeek, OpenAI legacy, most OpenAI-compatible
    // gateways) are precisely the class bot-hq itself stores in
    // `models.auth_token`: 32+ alphanumerics with no dash rules out the
    // `sk-ant-`/`sk-proj-` forms above and any hyphenated mention.
    if has_prefixed_token(body, "sk-", 32, |c| c.is_ascii_alphanumeric()) {
        return Some("an API key");
    }
    // Google API keys: `AIza` + 35 of [A-Za-z0-9_-].
    if has_prefixed_token(body, "AIza", 30, |c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Some("a Google API key");
    }
    // Stripe live secret / restricted keys.
    for prefix in ["sk_live_", "rk_live_"] {
        if has_prefixed_token(body, prefix, 20, |c| c.is_ascii_alphanumeric()) {
            return Some("a Stripe live key");
        }
    }
    // GitLab personal access tokens.
    if has_prefixed_token(body, "glpat-", 20, |c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Some("a GitLab access token");
    }
    // Hugging Face (`hf_` + 34) and npm (`npm_` + 36) tokens — the floors are
    // high enough that `hf_model` / `npm_config_x` identifiers never match.
    if has_prefixed_token(body, "hf_", 30, |c| c.is_ascii_alphanumeric()) {
        return Some("a Hugging Face token");
    }
    if has_prefixed_token(body, "npm_", 30, |c| c.is_ascii_alphanumeric()) {
        return Some("an npm token");
    }
    // A signed JWT: three base64url segments, the first two starting with
    // `eyJ` (`{"` encoded). A lone `eyJ…` is not enough — the second header
    // segment is the shape.
    if has_jwt(body) {
        return Some("a signed JWT");
    }
    // AWS SECRET access keys have no prefix; their shape is a 40-char base64
    // value beside the config key that names them.
    if has_aws_secret_assignment(body) {
        return Some("an AWS secret access key");
    }
    None
}

fn is_base64url(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// `eyJ<b64url>.eyJ<b64url>.<b64url>` with a signature of at least 10 chars.
fn has_jwt(body: &str) -> bool {
    body.match_indices("eyJ").any(|(i, _)| {
        let rest = &body[i..];
        let header: String = rest.chars().take_while(|c| is_base64url(*c)).collect();
        let after_header = &rest[header.len()..];
        let Some(payload_on) = after_header.strip_prefix('.') else {
            return false;
        };
        if !payload_on.starts_with("eyJ") {
            return false;
        }
        let payload: String = payload_on.chars().take_while(|c| is_base64url(*c)).collect();
        let Some(sig_on) = payload_on[payload.len()..].strip_prefix('.') else {
            return false;
        };
        header.len() >= 10
            && payload.len() >= 10
            && sig_on.chars().take_while(|c| is_base64url(*c)).count() >= 10
    })
}

/// `aws_secret_access_key` (any case) followed by `=` or `:` and a 40-char
/// base64 value. The bare config-key name in prose has no value beside it.
fn has_aws_secret_assignment(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.match_indices("aws_secret_access_key").any(|(i, needle)| {
        let after = &body[i + needle.len()..];
        let after = after.trim_start_matches(|c: char| c == ' ' || c == '\t' || c == '"' || c == '\'');
        let Some(value_on) = after.strip_prefix('=').or_else(|| after.strip_prefix(':')) else {
            return false;
        };
        let value_on = value_on.trim_start_matches(|c: char| c == ' ' || c == '\t' || c == '"' || c == '\'');
        let run = value_on
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '/' || *c == '+')
            .count();
        run == 40
    })
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

    /// A fixture body assembled at runtime from a vendor prefix and a tail,
    /// so that no secret-SHAPED literal sits in this file. The shapes are
    /// exactly what hosted push protection scans blobs for, and it does not
    /// know a fake from a real one: GitHub refused the repository's first
    /// public push over these fixtures (2026-09-09). The scanner under test
    /// only ever sees the joined string, so nothing about the check changes.
    fn fake(prefix: &str, tail: &str) -> String {
        format!("{prefix}{tail}")
    }

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
        // The `.env.<stage>` and direnv forms, missed before 1.0.5.
        for name in [".env.local", ".env.production", "app/.env.development", ".envrc"] {
            assert_eq!(scan_file(name, "X=1").map(|h| h.reason), Some("a .env file"), "{name}");
        }
        // By content — a key pasted into an ordinary note.
        let pem = fake("here is the key:\n-----BEGIN RSA ", "PRIVATE KEY-----\nMIIE…");
        assert_eq!(
            scan_file("notes.md", &pem).map(|h| h.reason),
            Some("a PEM private key block")
        );
        let aws_id = fake("AKIA", "IOSFODNN7EXAMPLE is the id");
        assert_eq!(
            scan_file("notes.md", &aws_id).map(|h| h.reason),
            Some("an AWS access key id")
        );
        let github = fake("token ghp_", "1234567890abcdefghijABCDEF");
        assert_eq!(
            scan_file("n.md", &github).map(|h| h.reason),
            Some("a GitHub access token")
        );
    }

    /// The families added after the 2026-09-06 sweep. Every fixture is
    /// clearly fake but correctly shaped once joined, so the matcher fires
    /// without a real secret — or a secret-shaped literal — ever living in
    /// this repo.
    #[test]
    fn the_sweeps_missing_families_are_caught_by_content() {
        let tail = "FAKETESTNOTREALSAMPLE";
        let cases: Vec<(String, &str)> = vec![
            (fake("key sk-", &format!("{tail}0123456789abcdef")), "an API key"),
            (fake("AIza", &format!("{tail}0123456789abcd-_x")), "a Google API key"),
            (fake("sk_live_", &format!("{tail}0123456789")), "a Stripe live key"),
            (fake("rk_live_", &format!("{tail}0123456789")), "a Stripe live key"),
            (fake("glpat-", &format!("{tail}01234")), "a GitLab access token"),
            (fake("hf_", &format!("{tail}0123456789abcdef")), "a Hugging Face token"),
            (fake("npm_", &format!("{tail}0123456789abcdefgh")), "an npm token"),
            (
                fake(
                    "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.",
                    "eyJzdWIiOiJmYWtlIn0.FAKESIGNATUREnotreal_0",
                ),
                "a signed JWT",
            ),
            (
                fake("aws_secret_access_key = ", &format!("{tail}wJalrXUtnFEMI/K7MDE")),
                "an AWS secret access key",
            ),
            (
                fake("AWS_SECRET_ACCESS_KEY: \"", &format!("{tail}wJalrXUtnFEMI/K7MDE\"")),
                "an AWS secret access key",
            ),
        ];
        for (body, reason) in &cases {
            assert_eq!(scan_file("n.md", body).map(|h| h.reason), Some(*reason), "{body}");
        }
    }

    /// …and the identifiers and prose those prefixes collide with stay clean:
    /// the length floors and the second-segment JWT shape are what make the
    /// difference.
    #[test]
    fn identifiers_that_share_a_prefix_are_not_secrets() {
        assert_eq!(
            scan_file(
                "notes.md",
                "Set hf_model_name and npm_config_registry in .env; the sk- prefix (sk-ant-, \
                 sk-proj-) marks Anthropic and OpenAI keys; glpat- tokens are GitLab's; \
                 AIza keys are Google's. The header segment eyJhbGciOiJIUzI1NiJ9 alone is \
                 not a JWT. The aws_secret_access_key config key is set in CI, not here."
            ),
            None
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
