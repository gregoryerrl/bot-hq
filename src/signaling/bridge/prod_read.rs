//! `prod_read` — a read-only Postgres query on the project's production
//! database, parked for the user's approval with its SQL on the card (feedback
//! #84; the user's pick, tray `afef06f0`; group K).
//!
//! Five production reads in one session used the same hand-written wrapper —
//! `source` an env file, `PGPASSWORD=… PGOPTIONS='-c
//! default_transaction_read_only=on …' psql -h <literal host> <<'SQL'` — with
//! read-only resting on that one option and on the agent writing only SELECTs,
//! the literal host in the command only so a keyword would fire, and the user
//! reviewing SQL wrapped in shell.
//!
//! Now the connection comes from the project's `policy.yaml` (`prod_read:`);
//! the SQL is checked before anything parks; the command bot-hq builds runs
//! the whole query as ONE transaction (`psql -1 -f -`) under
//! `default_transaction_read_only=on` with a statement timeout; the password
//! is read at approval time — parsed from the configured file, never sourced
//! — and handed to psql in its environment, so it is never in the text the
//! card shows or the transcript keeps (EYES, s-3158eb35). The approval row is
//! marked `exec_kind = prod_read` (0089) by this handler alone, so a command
//! made to look like one cannot be given the password.

use super::*;
use crate::policy::ProdReadConfig;

/// The heredoc delimiter the SQL rides in; a SQL that contains it on a line
/// of its own is refused.
const DELIMITER: &str = "BOTHQ_SQL";
const DEFAULT_TIMEOUT_MS: u32 = 30_000;
const MAX_TIMEOUT_MS: u32 = 300_000;
const MAX_SQL_BYTES: usize = 256 * 1024;

/// Statements `prod_read` runs: reads. `SET`, `BEGIN`, `COMMIT`, `END` and
/// `ROLLBACK` are not among them, so nothing can leave the read-only
/// transaction.
const READ_STATEMENTS: &[&str] = &["select", "with", "explain", "show", "table", "values"];

/// Check that `sql` only reads (EYES, s-3158eb35): every statement — split on
/// `;` outside quotes, dollar-quotes and comments — starts with one of
/// [`READ_STATEMENTS`]; a backslash outside quotes is allowed only on a line
/// that is wholly `\d…` or `\x` (so `SELECT 1 \g |cmd` and `\gexec` are
/// refused); and anything that cannot be followed (an unterminated quote,
/// dollar-quote or comment) is refused.
pub(crate) fn check_sql(sql: &str) -> Result<(), String> {
    if sql.trim().is_empty() {
        return Err("the SQL is empty".to_string());
    }
    if sql.lines().any(|l| l.trim() == DELIMITER) {
        return Err(format!("the SQL contains the line `{DELIMITER}`, which ends bot-hq's heredoc"));
    }
    let chars: Vec<char> = sql.chars().collect();
    let mut statements: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' | '"' => {
                // A quoted string or identifier; a doubled quote is an escape.
                let quote = c;
                current.push(c);
                i += 1;
                loop {
                    let Some(&d) = chars.get(i) else {
                        return Err("an unterminated quote".to_string());
                    };
                    current.push(d);
                    i += 1;
                    if d == quote {
                        if chars.get(i) == Some(&quote) {
                            current.push(quote);
                            i += 1;
                            continue;
                        }
                        break;
                    }
                }
                continue;
            }
            '$' => {
                // A dollar-quote: `$$…$$` or `$tag$…$tag$`.
                let rest: String = chars[i..].iter().collect();
                let tag_end = rest[1..].find('$').map(|p| p + 1);
                let tag = tag_end
                    .map(|e| &rest[..=e])
                    .filter(|t| t[1..t.len() - 1].chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
                if let Some(tag) = tag {
                    let body_start = tag.len();
                    let Some(close) = rest[body_start..].find(tag) else {
                        return Err("an unterminated dollar-quote".to_string());
                    };
                    let whole = &rest[..body_start + close + tag.len()];
                    current.push_str(whole);
                    i += whole.chars().count();
                    continue;
                }
            }
            '-' if chars.get(i + 1) == Some(&'-') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                let rest: String = chars[i + 2..].iter().collect();
                let Some(close) = rest.find("*/") else {
                    return Err("an unterminated /* comment".to_string());
                };
                i += 2 + rest[..close].chars().count() + 2;
                continue;
            }
            '\\' => {
                // Only a line that is wholly a describe or the expanded toggle.
                let line_start = current.rfind('\n').map(|p| p + 1).unwrap_or(0);
                let before = current[line_start..].trim();
                let line_end = chars[i..].iter().position(|&c| c == '\n').map(|p| i + p).unwrap_or(chars.len());
                let line: String = chars[i..line_end].iter().collect();
                let line = line.trim();
                let allowed = before.is_empty()
                    && line.matches('\\').count() == 1
                    && (line == "\\x" || line.starts_with("\\d"))
                    && !line.contains(['|', '!', '`', '>', '<']);
                if !allowed {
                    return Err(format!(
                        "a psql backslash command (`{}`) — only a whole-line `\\d…` or `\\x` is allowed",
                        line.split_whitespace().next().unwrap_or("\\")
                    ));
                }
                i = line_end;
                continue;
            }
            ';' => {
                statements.push(std::mem::take(&mut current));
                i += 1;
                continue;
            }
            _ => {}
        }
        current.push(c);
        i += 1;
    }
    statements.push(current);
    for statement in &statements {
        let first = statement
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .find(|t| !t.is_empty());
        let Some(first) = first else { continue };
        let keyword = first.to_ascii_lowercase();
        if !READ_STATEMENTS.contains(&keyword.as_str()) {
            return Err(format!(
                "a `{}` statement — prod_read runs only SELECT, WITH, EXPLAIN, SHOW, TABLE and VALUES",
                keyword.to_uppercase()
            ));
        }
    }
    Ok(())
}

/// The command approval runs (all values shell-quoted, the password NOT in
/// it): `psql -1 -f -` reads the heredoc as one script, in one transaction,
/// under `default_transaction_read_only=on` and a statement timeout.
pub(crate) fn build_command(cfg: &ProdReadConfig, sql: &str, timeout_ms: u32) -> String {
    let q = super::readback::sh_quote;
    let psql = cfg.psql.as_deref().filter(|p| !p.trim().is_empty()).unwrap_or("psql");
    let ssl = cfg
        .sslmode
        .as_deref()
        .filter(|m| !m.trim().is_empty())
        .map(|m| format!("PGSSLMODE={} ", q(m)))
        .unwrap_or_default();
    format!(
        "PGOPTIONS='-c default_transaction_read_only=on -c statement_timeout={timeout_ms} -c timezone=UTC' \
         {ssl}{} -1 -X -v ON_ERROR_STOP=1 -h {} -p {} -d {} -U {} -f - <<'{DELIMITER}'\n{}\n{DELIMITER}",
        q(psql),
        q(&cfg.host),
        cfg.port.unwrap_or(5432),
        q(&cfg.database),
        q(&cfg.user),
        sql.trim_end()
    )
}

/// The password, read now from the configured place: `password_file` (its
/// first line), or `password_var` in the dotenv `env_file` — parsed, not
/// sourced (EYES: sourcing runs the file as shell and exports every secret in
/// it). `KEY=value`, an optional `export `, `#` comments, and single- or
/// double-quoted values.
pub(crate) fn password(cfg: &ProdReadConfig) -> Result<String, String> {
    if let Some(file) = cfg.password_file.as_deref().filter(|f| !f.trim().is_empty()) {
        let text = std::fs::read_to_string(file).map_err(|e| format!("reading password_file `{file}`: {e}"))?;
        return Ok(text.lines().next().unwrap_or("").trim_end_matches('\r').to_string());
    }
    let (Some(file), Some(var)) = (
        cfg.env_file.as_deref().filter(|f| !f.trim().is_empty()),
        cfg.password_var.as_deref().filter(|v| !v.trim().is_empty()),
    ) else {
        return Err("prod_read has no password source: set `password_file`, or `env_file` and `password_var`".to_string());
    };
    let text = std::fs::read_to_string(file).map_err(|e| format!("reading env_file `{file}`: {e}"))?;
    for line in text.lines() {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else { continue };
        if key.trim() != var {
            continue;
        }
        let value = value.trim();
        let unquoted = match (value.chars().next(), value.chars().last()) {
            (Some('"'), Some('"')) | (Some('\''), Some('\'')) if value.len() >= 2 => &value[1..value.len() - 1],
            _ => value.split(" #").next().unwrap_or(value).trim_end(),
        };
        return Ok(unquoted.to_string());
    }
    Err(format!("`{var}` is not set in env_file `{file}`"))
}

impl SignalingBridge {
    /// The `prod_read` tool (group K): a read-only query on the project's
    /// production database, parked for the user — reviewed first when the
    /// executor asks, straight to the user when the reviewer does — with the
    /// SQL on the card. Refused, with why, before anything parks.
    pub async fn prod_read(
        &self,
        session_id: String,
        agent: String,
        sql: String,
        statement_timeout_ms: Option<u32>,
        approve_after: Option<String>,
    ) -> Result<String> {
        let policy = self.resolve_policy_for(&session_id).await?;
        let Some(cfg) = policy.prod_read else {
            anyhow::bail!(
                "prod_read is not configured for this session's project: the user adds a \
                 `prod_read:` block (engine: postgres, host, port, database, user, sslmode, \
                 env_file + password_var or password_file, statement_timeout_ms) to its policy.yaml"
            );
        };
        if !cfg.engine.trim().eq_ignore_ascii_case("postgres") {
            anyhow::bail!("prod_read supports `engine: postgres` only; this project sets `{}`", cfg.engine);
        }
        if cfg.host.trim().is_empty() || cfg.database.trim().is_empty() || cfg.user.trim().is_empty() {
            anyhow::bail!("the project's prod_read block needs a host, a database and a user");
        }
        if sql.len() > MAX_SQL_BYTES {
            anyhow::bail!("the SQL is {} bytes — prod_read takes at most 256 KB", sql.len());
        }
        if let Err(why) = check_sql(&sql) {
            anyhow::bail!("prod_read refused the SQL: {why}");
        }
        let timeout = statement_timeout_ms
            .or(cfg.statement_timeout_ms)
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .clamp(1_000, MAX_TIMEOUT_MS);
        let command = build_command(&cfg, &sql, timeout);
        let card = format!(
            "Production read (prod_read) on {}/{} as {} — one read-only transaction, {timeout} ms \
             statement timeout:\n```sql\n{}\n```",
            cfg.host,
            cfg.database,
            cfg.user,
            sql.trim_end()
        );
        let reviewer = self.session_reviewers(&session_id).iter().any(|slug| slug == &agent);
        let outcome = self
            .park_gated_command_as(&session_id, &agent, &command, approve_after.as_deref(), reviewer, Some(&card))
            .await?;
        // The mark that lets approval pass the password — written by this
        // handler alone (0089).
        let storage = self.storage.lock().await.clone();
        if let Some(storage) = storage {
            storage.set_tray_exec_kind(outcome.gate_id(), "prod_read").await?;
        }
        Ok(super::action_gate::park_outcome_text(&outcome, &command))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_reads_pass_the_sql_check() {
        for ok in [
            "select 1",
            "SELECT id FROM t WHERE note = 'a; b' ;",
            "with x as (select 1) select * from x;",
            "explain select * from t",
            "SHOW server_version",
            "table t; values (1), (2);",
            "select $$a;b$$, $tag$ x; y $tag$",
            "-- a comment; with a semicolon\nselect 1",
            "/* select; drop */ select 1",
            "\\d failed_jobs\nselect 1;",
            "\\x\nselect * from t limit 1",
            "select 'it''s' as q",
        ] {
            assert_eq!(check_sql(ok), Ok(()), "{ok}");
        }
        for (bad, why) in [
            ("delete from t", "`DELETE`"),
            ("select 1; delete from t", "`DELETE`"),
            ("SET default_transaction_read_only = off; select 1", "`SET`"),
            ("begin; select 1; commit", "`BEGIN`"),
            ("-- harmless\ndrop table t", "`DROP`"),
            ("select 1 \\g |rm -rf ~", "backslash"),
            ("select 1\n\\gexec", "backslash"),
            ("\\! ls", "backslash"),
            ("\\copy t to '/tmp/x'", "backslash"),
            ("select 'unterminated", "unterminated quote"),
            ("select $$x", "unterminated dollar-quote"),
            ("select 1 /* open", "unterminated"),
            ("select 1\nBOTHQ_SQL\nrm -rf ~", "BOTHQ_SQL"),
            ("", "empty"),
        ] {
            let err = check_sql(bad).expect_err(bad);
            assert!(err.contains(why), "{bad:?}: {err}");
        }
    }

    fn cfg() -> ProdReadConfig {
        ProdReadConfig {
            engine: "postgres".into(),
            host: "ep-solitary-field-x.aws.pg.laravel.cloud".into(),
            port: Some(5432),
            database: "main".into(),
            user: "readonly".into(),
            sslmode: Some("require".into()),
            ..ProdReadConfig::default()
        }
    }

    #[test]
    fn the_command_is_one_read_only_transaction_and_holds_no_password() {
        let command = build_command(&cfg(), "select 1;\n", 30_000);
        assert_eq!(
            command,
            "PGOPTIONS='-c default_transaction_read_only=on -c statement_timeout=30000 -c timezone=UTC' \
             PGSSLMODE='require' 'psql' -1 -X -v ON_ERROR_STOP=1 -h 'ep-solitary-field-x.aws.pg.laravel.cloud' \
             -p 5432 -d 'main' -U 'readonly' -f - <<'BOTHQ_SQL'\nselect 1;\nBOTHQ_SQL"
        );
        assert!(!command.contains("PGPASSWORD"));
    }

    #[test]
    fn the_password_is_parsed_from_the_env_file_never_sourced() {
        let dir = tempfile::tempdir().unwrap();
        let env = dir.path().join(".env.prod");
        std::fs::write(
            &env,
            "# prod\nexport OTHER=1\nDB_PASSWORD=\"p@ss $word with spaces\"\nPLAIN=abc # note\nSINGLE='x$y'\n",
        )
        .unwrap();
        let with = |var: &str| ProdReadConfig {
            env_file: Some(env.display().to_string()),
            password_var: Some(var.into()),
            ..cfg()
        };
        assert_eq!(password(&with("DB_PASSWORD")).unwrap(), "p@ss $word with spaces");
        assert_eq!(password(&with("PLAIN")).unwrap(), "abc");
        assert_eq!(password(&with("SINGLE")).unwrap(), "x$y");
        assert!(password(&with("MISSING")).unwrap_err().contains("not set"));
        let file = dir.path().join("pw");
        std::fs::write(&file, "s3cret\n").unwrap();
        let from_file = ProdReadConfig { password_file: Some(file.display().to_string()), ..cfg() };
        assert_eq!(password(&from_file).unwrap(), "s3cret");
        assert!(password(&cfg()).unwrap_err().contains("no password source"));
    }
}
