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

/// One secret-shaped stretch of a DECODED text: byte offsets (always on char
/// boundaries — every shape is ASCII) and what it is.
///
/// **Decoded, never serialized** (EYES 8eea5190). Run over JSON text, a span
/// can swallow the row's closing brace, and escapes shift the boundaries: a
/// token after an escaped `\n` sits behind an `n`, and a quoted value starts
/// at a backslash. Callers redact the strings BEFORE they serialize them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecretSpan {
    pub start: usize,
    pub end: usize,
    pub reason: &'static str,
}

/// Every secret-shaped span in `text`, in order, overlaps merged.
///
/// Self-identifying formats only — a literal vendor prefix plus a length
/// floor, a PEM header, a signed JWT, an AWS secret beside its config key, a
/// Laravel Sanctum token — which is what keeps prose about credentials clean
/// (see the module doc). The one scanner behind both the Context Library push
/// refusal ([`scan_file`]) and redaction ([`redact`]), so the two cannot
/// disagree about what a secret is.
pub fn find_secrets(text: &str) -> Vec<SecretSpan> {
    let mut spans = Vec::new();
    pem_spans(text, &mut spans);
    // AWS access key ids: `AKIA` + 16 uppercase alphanumerics — except AWS's
    // own documentation key, which ends `EXAMPLE` and is never a credential.
    prefixed_spans(text, "AKIA", 16, |c| c.is_ascii_uppercase() || c.is_ascii_digit(), "an AWS access key id", &mut spans, |run| {
        run.get(..16).is_some_and(|id| id.ends_with("EXAMPLE"))
    });
    // GitHub tokens — classic (`ghp_`/`gho_`/`ghs_`/`ghu_`) and fine-grained.
    for prefix in ["ghp_", "gho_", "ghs_", "ghu_", "github_pat_"] {
        prefixed_spans(text, prefix, 20, |c| c.is_ascii_alphanumeric() || c == '_', "a GitHub access token", &mut spans, |_| false);
    }
    // Anthropic / OpenAI-style keys.
    for prefix in ["sk-ant-", "sk-proj-"] {
        prefixed_spans(text, prefix, 20, |c| c.is_ascii_alphanumeric() || c == '-' || c == '_', "an API key", &mut spans, |_| false);
    }
    // Slack bot/user tokens.
    for prefix in ["xoxb-", "xoxp-", "xoxa-", "xoxs-"] {
        prefixed_spans(text, prefix, 10, |c| c.is_ascii_alphanumeric() || c == '-', "a Slack token", &mut spans, |_| false);
    }
    // The families the 2026-09-06 sweep found missing — each still a literal
    // vendor prefix plus a length floor, so a prose mention stays clean.
    // Bare `sk-` keys (DeepSeek, OpenAI legacy, most OpenAI-compatible
    // gateways) are precisely the class bot-hq itself stores in
    // `models.auth_token`: 32+ alphanumerics with no dash rules out the
    // `sk-ant-`/`sk-proj-` forms above and any hyphenated mention.
    prefixed_spans(text, "sk-", 32, |c| c.is_ascii_alphanumeric(), "an API key", &mut spans, |_| false);
    // Google API keys: `AIza` + 35 of [A-Za-z0-9_-].
    prefixed_spans(text, "AIza", 30, |c| c.is_ascii_alphanumeric() || c == '_' || c == '-', "a Google API key", &mut spans, |_| false);
    // Stripe live secret / restricted keys.
    for prefix in ["sk_live_", "rk_live_"] {
        prefixed_spans(text, prefix, 20, |c| c.is_ascii_alphanumeric(), "a Stripe live key", &mut spans, |_| false);
    }
    // GitLab personal access tokens.
    prefixed_spans(text, "glpat-", 20, |c| c.is_ascii_alphanumeric() || c == '_' || c == '-', "a GitLab access token", &mut spans, |_| false);
    // Hugging Face (`hf_` + 34) and npm (`npm_` + 36) tokens — the floors are
    // high enough that `hf_model` / `npm_config_x` identifiers never match.
    prefixed_spans(text, "hf_", 30, |c| c.is_ascii_alphanumeric(), "a Hugging Face token", &mut spans, |_| false);
    prefixed_spans(text, "npm_", 30, |c| c.is_ascii_alphanumeric(), "an npm token", &mut spans, |_| false);
    jwt_spans(text, &mut spans);
    aws_secret_spans(text, &mut spans);
    sanctum_spans(text, &mut spans);
    merge(spans)
}

/// `text` with every [`find_secrets`] span replaced by `[redacted: <what>]`.
///
/// Borrows when there is nothing to redact, so a clean string — the
/// overwhelming case, and the one exact-match callers depend on — comes back
/// byte-identical and unallocated. Idempotent: the marker holds no secret
/// shape.
pub fn redact(text: &str) -> std::borrow::Cow<'_, str> {
    let spans = find_secrets(text);
    if spans.is_empty() {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for span in spans {
        out.push_str(&text[at..span.start]);
        out.push_str("[redacted: ");
        out.push_str(span.reason);
        out.push(']');
        at = span.end;
    }
    out.push_str(&text[at..]);
    std::borrow::Cow::Owned(out)
}

/// [`redact`] for a string the caller owns: the same string, moved rather
/// than copied, when there is nothing to redact.
pub fn redact_string(text: String) -> String {
    let redacted = match redact(&text) {
        std::borrow::Cow::Owned(r) => Some(r),
        std::borrow::Cow::Borrowed(_) => None,
    };
    redacted.unwrap_or(text)
}

/// [`redact_string`], plus how many secrets it replaced — for a writer that
/// tells the agent its text was stored with markers in it.
pub fn redact_counting(text: String) -> (String, usize) {
    match find_secrets(&text).len() {
        0 => (text, 0),
        n => (redact_string(text), n),
    }
}

/// What a write's reply appends when [`redact_counting`] replaced `n > 0`
/// secrets, so the agent knows the stored text holds markers where it wrote
/// secrets — and matches the marker, not the secret, if it edits it later.
pub fn redaction_note(n: usize) -> String {
    format!(
        " — {n} secret-shaped string(s) in it were stored as `[redacted: …]` markers \
         (bot-hq redacts secrets in what agents write), so it holds the markers, not the secrets"
    )
}

/// The first secret in `body`, if any — what a push refusal names.
fn content_reason(body: &str) -> Option<&'static str> {
    find_secrets(body).first().map(|span| span.reason)
}

/// Sort and merge overlapping spans; the earliest-starting span names the
/// merged one.
fn merge(mut spans: Vec<SecretSpan>) -> Vec<SecretSpan> {
    spans.sort_by_key(|s| (s.start, std::cmp::Reverse(s.end)));
    let mut merged: Vec<SecretSpan> = Vec::with_capacity(spans.len());
    for span in spans {
        if let Some(last) = merged.last_mut() {
            if span.start < last.end {
                last.end = last.end.max(span.end);
                continue;
            }
        }
        merged.push(span);
    }
    merged
}

fn is_base64url(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// The end (byte offset) of the line starting at `from`: its `\n`, or the end.
fn line_end(text: &str, from: usize) -> usize {
    text[from..].find('\n').map_or(text.len(), |n| from + n)
}

/// A PEM private key block: from a `-----BEGIN … PRIVATE KEY-----` header line
/// to its `-----END … PRIVATE KEY-----` line. With no END line — a key cut
/// short, `head -5 key.pem` — to the last consecutive base64 line after the
/// header, never to the end of the text: whatever follows the key survives.
fn pem_spans(text: &str, out: &mut Vec<SecretSpan>) {
    for (begin, _) in text.match_indices("-----BEGIN") {
        let header_end = line_end(text, begin);
        if !text[begin..header_end].contains("PRIVATE KEY-----") {
            continue;
        }
        let end = match text[header_end..].find("-----END") {
            Some(n)
                if text[header_end + n..line_end(text, header_end + n)].contains("PRIVATE KEY-----") =>
            {
                line_end(text, header_end + n)
            }
            _ => {
                // Key lines may be indented (a key pasted into YAML), and an
                // encrypted key carries `Proc-Type:` / `DEK-Info:` header lines
                // and one blank line before its body (EYES, C1 review) — all
                // part of the block. A blank line counts only if key lines
                // follow it.
                let mut end = header_end;
                let mut blank_seen = false;
                // PEM headers come only BEFORE the body: after the first key
                // line, a `name: value` line is the text that follows (YAML).
                let mut body_started = false;
                while end < text.len() {
                    let next = end + 1;
                    let next_end = line_end(text, next);
                    let line = text[next..next_end].trim_end_matches('\r').trim_start();
                    let base64 = !line.is_empty()
                        && line.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=');
                    let pem_header = !body_started
                        && line.split_once(": ").is_some_and(|(name, _)| {
                            !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                        });
                    if base64 || pem_header {
                        end = next_end;
                        blank_seen = false;
                        body_started |= base64;
                    } else if line.is_empty() && !blank_seen {
                        // Tentative: taken only if the next line is key.
                        let after = line_end(text, (next_end + 1).min(text.len()));
                        let follows = text.get(next_end + 1..after).is_some_and(|l| {
                            let l = l.trim_end_matches('\r').trim_start();
                            !l.is_empty()
                                && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
                        });
                        if !follows {
                            break;
                        }
                        blank_seen = true;
                        end = next_end;
                    } else {
                        break;
                    }
                }
                end
            }
        };
        out.push(SecretSpan { start: begin, end, reason: "a PEM private key block" });
    }
}

/// `prefix` followed by at least `min_len` characters `accept` takes — the
/// length floor is what keeps a *mention* of a prefix in prose ("tokens
/// starting with `ghp_`") from reading as a token. `skip` sees the accepted
/// run and can exempt it (AWS's documentation key).
fn prefixed_spans(
    text: &str,
    prefix: &str,
    min_len: usize,
    accept: impl Fn(char) -> bool + Copy,
    reason: &'static str,
    out: &mut Vec<SecretSpan>,
    skip: impl Fn(&str) -> bool,
) {
    for (i, _) in text.match_indices(prefix) {
        let from = i + prefix.len();
        // Every accepted character is ASCII, so the count is the byte length.
        let len = text[from..].chars().take_while(|c| accept(*c)).count();
        if len >= min_len && !skip(&text[from..from + len]) {
            out.push(SecretSpan { start: i, end: from + len, reason });
        }
    }
}

/// `eyJ<b64url>.eyJ<b64url>.<b64url>`, each segment at least 10 characters.
fn jwt_spans(text: &str, out: &mut Vec<SecretSpan>) {
    for (i, _) in text.match_indices("eyJ") {
        let rest = &text[i..];
        let header = rest.chars().take_while(|c| is_base64url(*c)).count();
        let Some(payload_on) = rest[header..].strip_prefix('.') else {
            continue;
        };
        if !payload_on.starts_with("eyJ") {
            continue;
        }
        let payload = payload_on.chars().take_while(|c| is_base64url(*c)).count();
        let Some(sig_on) = payload_on[payload..].strip_prefix('.') else {
            continue;
        };
        let sig = sig_on.chars().take_while(|c| is_base64url(*c)).count();
        if header >= 10 && payload >= 10 && sig >= 10 {
            out.push(SecretSpan { start: i, end: i + header + 1 + payload + 1 + sig, reason: "a signed JWT" });
        }
    }
}

/// `aws_secret_access_key` (any case), then `=` or `:`, then a 40-char base64
/// value — the span is the VALUE; the config-key name stays readable.
fn aws_secret_spans(text: &str, out: &mut Vec<SecretSpan>) {
    let lower = text.to_ascii_lowercase();
    let pad = |s: &str| s.len() - s.trim_start_matches([' ', '\t', '"', '\'']).len();
    for (i, needle) in lower.match_indices("aws_secret_access_key") {
        let mut at = i + needle.len();
        at += pad(&text[at..]);
        if !(text[at..].starts_with('=') || text[at..].starts_with(':')) {
            continue;
        }
        at += 1;
        at += pad(&text[at..]);
        let run = text[at..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '/' || *c == '+')
            .count();
        if run == 40 {
            out.push(SecretSpan { start: at, end: at + 40, reason: "an AWS secret access key" });
        }
    }
}

/// A Laravel Sanctum API token — the Laravel Cloud token week 35 leaked
/// (F10): `<id>|` then 40 random alphanumerics, plus an 8-hex crc32b suffix
/// in Sanctum 4 (48). Mixed case is required, which keeps a
/// `<timestamp>|<40-hex sha>` git-log line out; the id must stand alone on
/// its left.
fn sanctum_spans(text: &str, out: &mut Vec<SecretSpan>) {
    for (bar, _) in text.match_indices('|') {
        let digits = text[..bar].chars().rev().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 || digits > 12 {
            continue;
        }
        let start = bar - digits;
        if text[..start].chars().next_back().is_some_and(|c| c.is_ascii_alphanumeric()) {
            continue;
        }
        let len = text[bar + 1..].chars().take_while(|c| c.is_ascii_alphanumeric()).count();
        let run = &text[bar + 1..bar + 1 + len];
        let mixed = run.chars().any(|c| c.is_ascii_uppercase()) && run.chars().any(|c| c.is_ascii_lowercase());
        if (40..=48).contains(&len) && mixed {
            out.push(SecretSpan { start, end: bar + 1 + len, reason: "a Laravel Sanctum API token" });
        }
    }
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
        let aws_id = fake("AKIA", "FAKETESTNOTREAL0 is the id");
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

    /// F10: every shape is found as a SPAN — exactly the secret — so redaction
    /// replaces it and keeps the text around it.
    #[test]
    fn each_shape_is_redacted_to_exactly_its_span() {
        let tail = "FAKETESTNOTREALSAMPLE";
        let cases: Vec<(String, &str)> = vec![
            (fake("AKIA", "FAKETESTNOTREAL0"), "an AWS access key id"),
            (fake("ghp_", "1234567890abcdefghijABCDEF"), "a GitHub access token"),
            (fake("sk-ant-", &format!("{tail}0123456789")), "an API key"),
            (fake("xoxb-", &format!("{tail}-0123")), "a Slack token"),
            (fake("sk-", &format!("{tail}0123456789abcdef")), "an API key"),
            (fake("AIza", &format!("{tail}0123456789abcd-_x")), "a Google API key"),
            (fake("sk_live_", &format!("{tail}0123456789")), "a Stripe live key"),
            (fake("glpat-", &format!("{tail}01234")), "a GitLab access token"),
            (fake("hf_", &format!("{tail}0123456789abcdef")), "a Hugging Face token"),
            (fake("npm_", &format!("{tail}0123456789abcdefgh")), "an npm token"),
            (
                fake("eyJhbGciOiJIUzI1NiJ9.", "eyJzdWIiOiJmYWtlIn0.FAKESIGNATUREnotreal_0"),
                "a signed JWT",
            ),
            (fake("1234|", &format!("{tail}0123456789abcdefGHIJKLMNOP")), "a Laravel Sanctum API token"),
        ];
        for (secret, reason) in &cases {
            let body = format!("before {secret} after");
            assert_eq!(
                redact(&body),
                format!("before [redacted: {reason}] after"),
                "{secret}"
            );
        }
        // An AWS secret keeps its config-key name; only the value goes.
        let body = fake("aws_secret_access_key = ", &format!("{tail}wJalrXUtnFEMI/K7MDE\ntail"))
            .replace("\\n", "\n");
        assert_eq!(
            redact(&body),
            "aws_secret_access_key = [redacted: an AWS secret access key]\ntail"
        );
    }

    /// F10: the Sanctum shape the week-35 Laravel Cloud token had — `<id>|` then
    /// 40 random alphanumerics, 48 with Sanctum 4's crc suffix — caught at the
    /// start of a line after output (the case a JSON-escaped scan missed,
    /// EYES 8eea5190); a git-log `<timestamp>|<sha>` and an id glued to a word
    /// are not.
    #[test]
    fn a_sanctum_token_is_caught_and_its_lookalikes_are_not() {
        let forty = fake("AbCdEfGhIjKlMnOpQrSt", "UvWxYz01234567890abc");
        let forty_eight = fake(&forty, "0a1b2c3d");
        assert_eq!((forty.len(), forty_eight.len()), (40, 48), "the fixture lengths");
        for token in [format!("7|{forty}"), format!("4412|{forty_eight}")] {
            let body = format!("ok\n{token}\nnext");
            assert_eq!(redact(&body), "ok\n[redacted: a Laravel Sanctum API token]\nnext", "{token}");
        }
        let sha = "3ae21c914b3139f71c63a0406a92be6cfdff3a38";
        assert!(find_secrets(&format!("1758870000|{sha}")).is_empty(), "a lowercase hex sha is not a token");
        assert!(find_secrets(&format!("row9|{forty}")).is_empty(), "the id must stand alone");
        assert!(find_secrets(&format!("7|{forty}xyz0123456789")).is_empty(), "longer than 48 is not the shape");
    }

    /// AWS's own documentation key id is never a credential (EYES: every
    /// session-doc hit in this install was it).
    #[test]
    fn the_aws_documentation_key_is_not_a_secret() {
        assert!(find_secrets(&fake("AKIA", "IOSFODNN7EXAMPLE")).is_empty());
        assert_eq!(scan_file("notes.md", &fake("AKIA", "IOSFODNN7EXAMPLE")), None);
    }

    /// A key cut short (`head -5 key.pem`) is redacted through its last base64
    /// line — never to the end of the text (EYES 8eea5190: that would swallow
    /// whatever follows). With an END line, the block ends there.
    #[test]
    fn a_pem_block_is_redacted_to_its_end_or_its_last_key_line() {
        let head = fake("-----BEGIN RSA ", "PRIVATE KEY-----");
        let cut = format!("{head}\nMIIEpAIBAAKCAQEA0fake+/line==\nMIIEpQIBAAKCAQEAfake2\nAAAA\n3 lines shown");
        assert_eq!(redact(&cut), "[redacted: a PEM private key block]\n3 lines shown");
        let end = fake("-----END RSA ", "PRIVATE KEY-----");
        let whole = format!("key:\n{head}\nMIIEfake\n{end}\nafter the key");
        assert_eq!(redact(&whole), "key:\n[redacted: a PEM private key block]\nafter the key");
    }

    /// …and a cut-short key that is ENCRYPTED (`Proc-Type:`/`DEK-Info:` lines
    /// and a blank line before the body) or INDENTED (pasted into YAML) is
    /// redacted through its body too (EYES, C1 review); what follows survives.
    #[test]
    fn an_encrypted_or_indented_cut_key_is_redacted_through_its_body() {
        let head = fake("-----BEGIN RSA ", "PRIVATE KEY-----");
        let encrypted = format!(
            "{head}\nProc-Type: 4,ENCRYPTED\nDEK-Info: AES-128-CBC,0A1B2C\n\nMIIEfakebody+/==\nMIIEfake2\nshown 5 lines"
        );
        assert_eq!(redact(&encrypted), "[redacted: a PEM private key block]\nshown 5 lines");
        let yaml = format!("key: |\n  {head}\n  MIIEfakebody+/==\n  MIIEfake2\nnext: value");
        assert_eq!(redact(&yaml), "key: |\n  [redacted: a PEM private key block]\nnext: value");
        // A blank line NOT followed by key lines is not swallowed.
        let gap = format!("{head}\nMIIEfake\n\nprose after a gap");
        assert_eq!(redact(&gap), "[redacted: a PEM private key block]\n\nprose after a gap");
    }

    #[test]
    fn redaction_borrows_clean_text_and_is_idempotent() {
        let clean = "no secrets here, only ghp_ and sk- mentioned";
        assert!(matches!(redact(clean), std::borrow::Cow::Borrowed(_)));
        let dirty = format!("a {} b", fake("ghp_", "1234567890abcdefghijABCDEF"));
        let once = redact(&dirty).into_owned();
        assert_eq!(redact(&once), once);
    }

    #[test]
    fn overlapping_spans_merge_under_the_first() {
        let s = |start, end, reason| SecretSpan { start, end, reason };
        assert_eq!(
            merge(vec![s(20, 25, "c"), s(5, 15, "b"), s(0, 10, "a")]),
            vec![s(0, 15, "a"), s(20, 25, "c")]
        );
    }

    /// **Per-row redaction relies on every row being a WHOLE block** (EYES,
    /// F10 point 4): claude-code is spawned without partial messages, so a
    /// secret cannot be split across two rows and slip past both. Adding the
    /// flag must revisit this.
    #[test]
    fn claude_code_is_spawned_without_partial_messages() {
        let spawn = include_str!("../agents/spawn.rs");
        let flag = format!("--include-{}-messages", "partial");
        assert!(!spawn.contains(&flag), "partial messages would split a secret across rows");
    }

    /// A generous bound, not a benchmark: `post_to_channel` redacts every
    /// plain row, and a tool result can carry megabytes.
    #[test]
    fn a_megabyte_of_clean_text_scans_quickly() {
        let body = "a | b | c - plain prose, sk- and ghp_ mentioned, 1234 | table cell\n".repeat(16_000);
        assert!(body.len() > 1_000_000);
        let started = std::time::Instant::now();
        assert!(find_secrets(&body).is_empty());
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "{:?}", started.elapsed());
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

    /// `redact_counting` reports how many secrets it replaced, and leaves a
    /// clean string as it was.
    #[test]
    fn redact_counting_counts_what_it_replaced() {
        let gh = fake("ghp_", "1234567890abcdefghijABCDEF");
        assert_eq!(redact_counting("clean".to_string()), ("clean".to_string(), 0));
        let (out, n) = redact_counting(format!("{gh} and {gh}"));
        assert_eq!(n, 2);
        assert_eq!(out, "[redacted: a GitHub access token] and [redacted: a GitHub access token]");
    }
}
