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
//! the SQL is checked before anything parks, scanned as PostgreSQL scans it
//! and refused wherever the two could disagree; the command bot-hq builds runs
//! the whole query as ONE transaction (`psql -1 -f -`) that bot-hq's own
//! first statements make read-only, with a statement timeout, before the
//! agent's SQL runs. That stops a write the SQL spells out; a function with
//! side effects outside the transaction (`dblink`, `pg_terminate_backend`) is
//! the database role's to refuse, so a read-only role is the real boundary.
//! The password is read at approval time — parsed from the configured file,
//! never sourced — and handed to psql in its environment, so it is never in
//! the text the card shows or the transcript keeps (EYES, s-3158eb35). The
//! approval row is marked `exec_kind = prod_read` (0089) by this handler
//! alone, so a command made to look like one cannot be given the password.

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

/// A character PostgreSQL can start a name with: a letter, `_`, or any
/// non-ASCII character (its bytes are `\200-\377` to PostgreSQL's lexer).
fn ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || !c.is_ascii()
}

/// A character that continues a name. `$` is one, so `a$b$` is a single name
/// to PostgreSQL and never opens a dollar-quote.
fn ident_cont(c: char) -> bool {
    ident_start(c) || c.is_ascii_digit() || c == '$'
}

/// Check that `sql` only reads (EYES, s-3158eb35). The SQL is scanned the way
/// PostgreSQL and psql scan it — names (`$` included), numbers, quoted
/// strings and names, dollar-quotes, `--` and `/* */` comments — and split on
/// the `;` outside them, and every statement must start with one of
/// [`READ_STATEMENTS`]. Where this scan and PostgreSQL's could disagree about
/// where a string or comment ends, the SQL is refused instead (EYES
/// `1e62db22`: a disagreement hides a statement): a `/*` inside a comment
/// (PostgreSQL nests them), an `E'…'` or `U&'…'` string, a backslash anywhere
/// but a whole-line `\d…` or `\x`, a `$` that opens no dollar-quote, a `$`
/// right after a number, and a carriage return. Anything unterminated is
/// refused too.
pub(crate) fn check_sql(sql: &str) -> Result<(), String> {
    if sql.trim().is_empty() {
        return Err("the SQL is empty".to_string());
    }
    if sql.contains('\r') {
        return Err("a carriage return — end lines with a plain newline".to_string());
    }
    if sql.contains('\0') {
        return Err("a NUL character".to_string());
    }
    if sql.lines().any(|l| l.trim() == DELIMITER) {
        return Err(format!("the SQL contains the line `{DELIMITER}`, which ends bot-hq's heredoc"));
    }
    let chars: Vec<char> = sql.chars().collect();
    let no_backslash = |text: &[char], what: &str| {
        if text.contains(&'\\') {
            Err(format!("a backslash inside {what}"))
        } else {
            Ok(())
        }
    };
    let mut statements: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if ident_start(c) {
            let start = i;
            while i < chars.len() && ident_cont(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            if word.eq_ignore_ascii_case("e") && chars.get(i) == Some(&'\'') {
                return Err("an E'…' string, whose backslash escapes this check does not follow".to_string());
            }
            if word.eq_ignore_ascii_case("u")
                && chars.get(i) == Some(&'&')
                && matches!(chars.get(i + 1), Some('\'' | '"'))
            {
                return Err("a U&'…' string or U&\"…\" name".to_string());
            }
            current.push_str(&word);
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '.') {
                i += 1;
            }
            if chars.get(i) == Some(&'$') {
                return Err("a `$` right after a number".to_string());
            }
            current.extend(&chars[start..i]);
            continue;
        }
        match c {
            '\'' | '"' => {
                // A quoted string or name; a doubled quote is an escape.
                let quote = c;
                let start = i;
                i += 1;
                loop {
                    let Some(&d) = chars.get(i) else {
                        return Err("an unterminated quote".to_string());
                    };
                    i += 1;
                    if d == quote {
                        if chars.get(i) == Some(&quote) {
                            i += 1;
                            continue;
                        }
                        break;
                    }
                }
                no_backslash(&chars[start..i], "a quoted string or name")?;
                current.extend(&chars[start..i]);
                continue;
            }
            '$' => {
                // At the start of a token: `$$…$$` or `$tag$…$tag$`, the tag
                // shaped like a name without `$`. A `$1` or a lone `$` is
                // refused.
                let mut end = i + 1;
                if chars.get(end).is_some_and(|&t| ident_start(t)) {
                    while chars.get(end).is_some_and(|&t| ident_start(t) || t.is_ascii_digit()) {
                        end += 1;
                    }
                }
                if chars.get(end) != Some(&'$') {
                    return Err("a `$` that opens no dollar-quote (prod_read takes no `$1` parameters)".to_string());
                }
                let tag = &chars[i..=end];
                let body = end + 1;
                let Some(close) = chars[body..].windows(tag.len()).position(|w| w == tag) else {
                    return Err("an unterminated dollar-quote".to_string());
                };
                let stop = body + close + tag.len();
                no_backslash(&chars[i..stop], "a dollar-quoted string")?;
                current.extend(&chars[i..stop]);
                i = stop;
                continue;
            }
            '-' if chars.get(i + 1) == Some(&'-') => {
                let start = i;
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                no_backslash(&chars[start..i], "a comment")?;
                continue;
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                // Scanned left to right as PostgreSQL does: a `/*` before the
                // first `*/` would open a nested comment.
                let mut j = i + 2;
                loop {
                    match (chars.get(j), chars.get(j + 1)) {
                        (None, _) | (_, None) => return Err("an unterminated /* comment".to_string()),
                        (Some('/'), Some('*')) => {
                            return Err("a /* inside a /* comment — PostgreSQL nests them".to_string());
                        }
                        (Some('*'), Some('/')) => break,
                        (Some('\\'), _) => return Err("a backslash inside a comment".to_string()),
                        _ => j += 1,
                    }
                }
                i = j + 2;
                continue;
            }
            '\\' => {
                // Only a line that is wholly a describe or the expanded
                // toggle, made of plain characters: no `;`, quote, `:` or
                // second backslash psql could read more into.
                let line_start = current.rfind('\n').map(|p| p + 1).unwrap_or(0);
                let before = current[line_start..].trim();
                let line_end = chars[i..].iter().position(|&c| c == '\n').map(|p| i + p).unwrap_or(chars.len());
                let line: String = chars[i..line_end].iter().collect();
                let line = line.trim();
                let allowed = before.is_empty()
                    && (line.starts_with("\\d") || line.starts_with("\\x"))
                    && line[1..]
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '*' | '?' | '+' | ' ' | '\t'));
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

/// What runs first inside `psql -1`'s transaction, before the agent's SQL:
/// the transaction made read-only, its time limit and time zone, then one
/// query. After a query PostgreSQL refuses to make a transaction read-write
/// ("must be set before any query"), so no statement of the agent's can, and
/// none of it rests on `PGOPTIONS`, which a connection pooler may drop (EYES
/// `1e62db22`). The query's one-row result opens the output.
fn guard(timeout_ms: u32) -> String {
    format!(
        "SET TRANSACTION READ ONLY;\nSET LOCAL statement_timeout = {timeout_ms};\nSET LOCAL timezone = 'UTC';\n\
         SELECT current_setting('transaction_read_only') AS bot_hq_read_only;"
    )
}

/// The command approval runs (all values shell-quoted, the password NOT in
/// it): `psql -1 -f -` reads the heredoc as one script, in one transaction —
/// [`guard`] first, then the agent's SQL — also under
/// `default_transaction_read_only=on`, quiet (`-q`: no `SET` tags), and in
/// UTF-8, the encoding [`check_sql`] scanned.
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
         PGCLIENTENCODING=UTF8 {ssl}{} -1 -X -q -v ON_ERROR_STOP=1 -h {} -p {} -d {} -U {} -f - <<'{DELIMITER}'\n\
         {}\n{}\n{DELIMITER}",
        q(psql),
        q(&cfg.host),
        cfg.port.unwrap_or(5432),
        q(&cfg.database),
        q(&cfg.user),
        guard(timeout_ms),
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
        // Windows line ends become plain ones, so the SQL checked is the SQL
        // run; a lone carriage return is refused by the check.
        let sql = sql.replace("\r\n", "\n");
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
            "select a$b$ from t",
            "select café, 1e5, 1.5, t.x from t",
            "/**/ select 1",
            "\\dt public.*\nselect 1",
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
            // EYES `1e62db22`: where PostgreSQL would read a string or comment
            // ending elsewhere than this scan, the SQL is refused.
            ("/* /* */ select ' */ drop table t; -- '", "inside a /* comment"),
            ("/*/*/ select ' */ */ drop table t; -- '", "inside a /* comment"),
            ("select E'\\' \" ' ; drop table t ; -- \"", "E'"),
            ("select a$b$ from t; drop table t2; -- $b$", "`DROP`"),
            ("select U&'x'", "U&"),
            ("select 'a\\b'", "backslash inside a quoted string"),
            ("select $tag$ a\\b $tag$", "backslash inside a dollar-quoted string"),
            ("-- a \\ b\nselect 1", "backslash inside a comment"),
            ("/* a \\ b */ select 1", "backslash inside a comment"),
            ("select $1", "opens no dollar-quote"),
            ("select 1$x", "after a number"),
            ("select 1\rselect 2", "carriage return"),
            ("\\dt t; drop table t", "backslash command"),
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
             PGCLIENTENCODING=UTF8 PGSSLMODE='require' 'psql' -1 -X -q -v ON_ERROR_STOP=1 \
             -h 'ep-solitary-field-x.aws.pg.laravel.cloud' -p 5432 -d 'main' -U 'readonly' -f - <<'BOTHQ_SQL'\n\
             SET TRANSACTION READ ONLY;\nSET LOCAL statement_timeout = 30000;\nSET LOCAL timezone = 'UTC';\n\
             SELECT current_setting('transaction_read_only') AS bot_hq_read_only;\nselect 1;\nBOTHQ_SQL"
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
