//! sqlite layer. `Storage` owns a `SqlitePool`; all queries are async via
//! sqlx. The query methods are split across per-table submodules, each
//! contributing its own `impl Storage` block:
//!
//! - one per table family: `sessions`, `messages`, `participants` (the
//!   roster, cursors, deliveries, phase votes), `tray`, `findings`,
//!   `session_docs`, `projects`, `cl_index` / `cl_atoms`, `models` /
//!   `agent_config`, `plugins`, `context_readings`, `retrieval_events`,
//!   `gc`, … — read the `mod` list below, not this sentence, for the set
//!
//! This module keeps the `Storage` struct, the `open`/`memory` constructors,
//! the pool accessor, and the shared `cl_search_table` generic used by the
//! CL index/folder searches.

use anyhow::{Context, Result};
use sha2::{Digest, Sha384};
use sqlx::migrate::{Migration, Migrator};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;
use std::sync::LazyLock;

pub mod row_types;

mod activity_events;
mod agent_config;
mod cancel_events;
mod cl_atoms;
mod cl_index;
mod context_readings;
mod feedback;
mod findings;
mod gc;
mod messages;
mod models;
mod participants;
mod plugin_kv;
mod plugins;
mod projects;
mod retrieval_events;
mod session_docs;
mod sessions;
mod time;
mod tray;

pub use cl_atoms::{Atom, RetrievedAtom};
pub use context_readings::ContextReading;
pub(crate) use cl_atoms::estimate_tokens;
pub use models::{DEFAULT_SPAWN_MODEL_NAME, WORKTREE_DEFAULT_KEY};
pub use cancel_events::CancelEventRecord;
pub use participants::{
    participant_display_name, participant_slug, MODE_ACTIVE, render_wire, speaker_of, ChannelPage, Envelope,
    Participant,
    ParticipantDraft, PersistedMessage, Role, RoleDraft, MAX_SESSION_PARTICIPANTS,
    PARTICIPATION_MODES, UNREAD_BATCH_LIMIT, WIRE_BODY_CLAMP_BYTES, WIRE_JOIN,
};
pub use feedback::{FEEDBACK_KINDS, FEEDBACK_STATUSES};
pub use tray::{is_gate_options, is_gate_row, GATE_OPTIONS_JSON};
pub use findings::{FindingUidResolution, OPEN_BLOCKING_FOR_SESSION};
pub use row_types::{
    AgentConfig, AgentFeedback, CancelEvent, ClFolder, ClIndexEntry, ClRead,
    Finding, FindingSeverity, FindingStatus, Message, MessageKind, Model, Plugin, Project,
    QuestionKind, RetrievalStats, Session, SessionDocument, SessionTrayEntry, SessionWithPreview,
};
pub(crate) use time::{cutoff_days_ago, now_utc};
pub use messages::{messages_tail_sql, participant_text_since_sql};

/// A user's search text as a `LIKE` pattern: `%…%`, lower-cased, with the
/// pattern metacharacters escaped so `%`, `_` and `\` in the input match
/// themselves. Every `LIKE ?` fed by this must carry `ESCAPE '\\'`. Without it
/// a search for `_` or `%` matched every row and `foo_bar` matched `fooXbar`
/// (round 8, N3).
pub(crate) fn like_pattern(query: &str) -> String {
    let mut out = String::with_capacity(query.len() + 2);
    out.push('%');
    for ch in query.to_lowercase().chars() {
        if matches!(ch, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('%');
    out
}

/// The migration set exactly as `sqlx::migrate!` embeds it: each `.sql`
/// file's RAW bytes from the checkout that built this binary, line endings
/// and all. Private on purpose — every consumer goes through [`MIGRATOR`],
/// which is this set with the line endings taken out of the checksum. This is
/// the one place in the crate that spells `sqlx::migrate!`.
static EMBEDDED_MIGRATIONS: Migrator = sqlx::migrate!("./migrations");

/// The embedded migrations with every `sql` normalised to LF, so that a
/// migration's checksum is a property of its TEXT and not of the line endings
/// of the checkout that built the binary. Both `Storage::open` and
/// `memory_bare` run this and nothing else.
///
/// **Why (the 1.0.0 → 1.0.1 Windows brick).** `sqlx::migrate!` embeds each
/// file's raw bytes and stamps `sha384(bytes)` into `_sqlx_migrations` when
/// it applies the migration; on every later open the migrator refuses to
/// start if a stored checksum differs from the embedded one ("migration N was
/// previously applied but has been modified"). Every Windows build through
/// 1.0.0 came off a GitHub `windows-latest` runner with `core.autocrlf=true`
/// and no `.gitattributes`, so it embedded CRLF text and stamped CRLF
/// checksums into every Windows user's database. `.gitattributes`
/// (`* text=auto eol=lf`) landed before 1.0.1, so that build embedded LF —
/// and exited within a second of launch, before any window, on every
/// upgraded Windows install, although not one migration had changed (all 75
/// applied rows on the reporting machine matched the CRLF digest, none the
/// LF one). Fresh installs and macOS/Linux never saw it. Normalising here
/// makes the checksum the same whichever way the file was checked out;
/// [`repair_crlf_migration_checksums`] brings the databases the CRLF builds
/// stamped forward to it. The codebase already knew `include_str!` is
/// checkout-dependent — see `lf()` in `participants.rs` — for its TESTS; this
/// was the first production consequence.
///
/// Built by pushing each [`Migration`] back through `Migration::new`, which is
/// what computes the checksum. The `Migrator`/`Migration` fields this reads
/// and writes (`migrations`, `ignore_missing`, `locking`, `no_tx`;
/// `version`, `description`, `migration_type`, `sql`, `no_tx`, `checksum`)
/// are `pub` but `#[doc(hidden)]` and
/// semver-exempt in sqlx 0.8 — `migrate!` itself needs them public — and
/// Cargo.lock pins 0.8.6. The test
/// `migration_checksum_migrator_is_lf_normalised_and_sha384_of_its_sql` pins
/// the digest and that nothing but the line endings changed.
pub(crate) static MIGRATOR: LazyLock<Migrator> = LazyLock::new(|| {
    let migrations: Vec<Migration> = EMBEDDED_MIGRATIONS
        .iter()
        .map(|m| {
            Migration::new(
                m.version,
                m.description.clone(),
                m.migration_type,
                Cow::Owned(m.sql.replace("\r\n", "\n")),
                m.no_tx,
            )
        })
        .collect();
    // The three flags `migrate!` set alongside the set are COPIED, not
    // assumed equal to `Migrator::DEFAULT`: a 0.8.x expansion that set one
    // (say `no_tx` from a directory marker) would otherwise diverge silently.
    Migrator {
        migrations: Cow::Owned(migrations),
        ignore_missing: EMBEDDED_MIGRATIONS.ignore_missing,
        locking: EMBEDDED_MIGRATIONS.locking,
        no_tx: EMBEDDED_MIGRATIONS.no_tx,
    }
});

/// `sha384` of the CRLF form of an LF-normalised migration text — the
/// checksum a build from a CRLF checkout (bot-hq ≤ 1.0.0 on Windows) stamped
/// for it. Uses the `sha2` crate directly rather than a throwaway
/// `Migration::new`: it is the very digest sqlx computes
/// (`Sha384::digest(sql.as_bytes())`, pinned by the `migration_checksum_…`
/// test on [`MIGRATOR`]), it says what it hashes, and it is the exact
/// computation that was checked against the field database.
fn crlf_checksum(lf_sql: &str) -> Vec<u8> {
    Sha384::digest(lf_sql.replace('\n', "\r\n").as_bytes()).to_vec()
}

/// Rewrite the `_sqlx_migrations` checksums a CRLF-checkout build stamped to
/// the LF digests [`MIGRATOR`] carries, so the migrator's mismatch check
/// passes for a migration whose TEXT has not changed. Returns how many rows
/// it rewrote.
///
/// For every embedded migration whose applied (`success = 1`) row holds a
/// checksum other than the LF one, the row is rewritten if — and only if —
/// what it holds is the sha384 of the CRLF form of the same text. Anything
/// else is left exactly as found, so the migrator still refuses a migration
/// that was genuinely edited after it was applied. A database with no
/// `_sqlx_migrations` table yet (a fresh install) is left for the migrator to
/// create; this never creates the table itself. All rewrites land in one
/// transaction, and an error here is propagated: if this table cannot be
/// read, the migrator that runs next could not read it either.
///
/// Not gated on the platform: whether a row needs this is a property of the
/// database file and the build that wrote it, not of the OS opening it now.
/// It runs BEFORE the migrator in [`Storage::open`], necessarily — the
/// migrator is what would fail.
pub(crate) async fn repair_crlf_migration_checksums(pool: &SqlitePool) -> Result<usize> {
    let table: Option<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_optional(pool)
    .await
    .context("looking for the _sqlx_migrations table")?;
    if table.is_none() {
        return Ok(0);
    }
    let mut tx = pool
        .begin()
        .await
        .context("beginning the migration checksum repair")?;
    let applied: HashMap<i64, Vec<u8>> = sqlx::query_as::<_, (i64, Vec<u8>)>(
        "SELECT version, checksum FROM _sqlx_migrations WHERE success = 1",
    )
    .fetch_all(&mut *tx)
    .await
    .context("reading the applied migration checksums")?
    .into_iter()
    .collect();
    let mut repaired = 0;
    for migration in MIGRATOR.iter() {
        let Some(stored) = applied.get(&migration.version) else {
            continue;
        };
        let lf: &[u8] = &migration.checksum;
        if stored.as_slice() == lf || stored.as_slice() != crlf_checksum(&migration.sql) {
            continue;
        }
        sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = ?")
            .bind(lf)
            .bind(migration.version)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("rewriting the checksum of migration {}", migration.version))?;
        repaired += 1;
    }
    tx.commit()
        .await
        .context("committing the migration checksum repair")?;
    Ok(repaired)
}

#[derive(Clone)]
pub struct Storage {
    pool: SqlitePool,
}

impl Storage {
    /// Open the database at `db_path`, run migrations, return a ready Storage.
    /// Creates the file if missing. The parent directory must already exist.
    pub async fn open(db_path: &Path) -> Result<Self> {
        let dsn = format!("sqlite://{}", db_path.display());
        let opts = SqliteConnectOptions::from_str(&dsn)
            .with_context(|| format!("invalid sqlite dsn: {dsn}"))?
            .create_if_missing(true)
            .foreign_keys(true)
            // **WAL, for the reader this app does not own** (F1). The journal
            // mode was `delete`, which takes an exclusive lock for the length of
            // every write — against an 8-connection pool AND the git hooks,
            // which open this same file read-only from a SEPARATE PROCESS
            // (`hooks.rs::check_findings_gate`). Under `delete` a commit's gate
            // check can be locked out by an ordinary write; under WAL readers
            // never block writers and writers never block readers.
            //
            // Set on the connection, persisted in the FILE HEADER: the first
            // connection flips the database and every later opener — including
            // the hook's read-only handle, which cannot set it itself — inherits
            // it. Reverting means opening the file once with `journal_mode` back
            // to delete; nothing in a migration can express it.
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(opts)
            .await
            .with_context(|| format!("opening sqlite at {}", db_path.display()))?;
        // Checksums stamped by a CRLF-checkout build, brought forward BEFORE
        // the migrator compares them — see `MIGRATOR` for the 1.0.1 story.
        match repair_crlf_migration_checksums(&pool)
            .await
            .context("repairing migration checksums stored by a CRLF-checkout build")?
        {
            0 => {}
            n => tracing::info!(
                rows = n,
                "repaired migration checksums: they were stored by a build from a CRLF checkout \
                 (bot-hq 1.0.0 or earlier on Windows); this build embeds the migrations with LF \
                 line endings, so the stored digests were rewritten to the LF ones"
            ),
        }
        MIGRATOR
            .run(&pool)
            .await
            .context("running sqlite migrations")?;
        let storage = Self { pool };
        // Windows: rewrite legacy `\`-keyed CL rows to the portable `/` form.
        //
        // ORDERING IS LOAD-BEARING, and it is why this lives here rather than
        // in main.rs's startup sequence: every walk needs a `Storage`, so
        // running it inside `open` makes it structurally impossible for the
        // startup rescan loop or the fs-watcher's first tick to reach
        // `walk_cl_dir` first. If either did, every `\` key would look like an
        // orphan and be purged — destroying exactly the `agent_visible` flag
        // this migration exists to preserve. A comment could state that
        // constraint; placing it here enforces it.
        //
        // Gated at the CALL SITE, not inside the function: on Unix `\` is a
        // legal filename character, so rewriting those keys there would corrupt
        // them. Keeping the function itself platform-independent keeps it
        // testable on every platform.
        #[cfg(windows)]
        {
            match storage.normalize_backslash_cl_keys().await {
                Ok(0) => {}
                Ok(n) => tracing::info!(rows = n, "normalized legacy backslash CL keys"),
                // Non-fatal: on failure the old keys simply remain, which is
                // the pre-existing state — it must not brick app startup.
                Err(e) => tracing::warn!(?e, "backslash CL key normalization failed"),
            }
        }
        Ok(storage)
    }

    /// In-memory test backend. Available to integration tests in `tests/`.
    /// The TEST world: a migrated in-memory DB **with the example pair
    /// installed** (1.0.0 Batch 4). Migration 0072 made a truly fresh DB
    /// carry only the neutral `agent` role — the right product default, but
    /// the wrong fixture for the hundreds of tests modelling a two-role
    /// (executor + reviewer) session; they model a user who installed the
    /// preset, and this says so once instead of in every test. Fresh-install
    /// behavior itself is asserted against [`Self::memory_bare`].
    pub async fn memory() -> Result<Self> {
        let s = Self::memory_bare().await?;
        s.install_role_preset()
            .await
            .context("installing the example pair into the test DB")?;
        // Retire the neutral `agent` role in the TEST world: it seeds first
        // (lowest surviving roles.id), so leaving it active would make every
        // default roster `agent` + `hands` instead of the pair the fixtures
        // model. Archived, not deleted — exactly what a real user who
        // installed the pair and retired the default would have.
        sqlx::query("UPDATE roles SET archived = 1 WHERE slug = 'agent'")
            .execute(&s.pool)
            .await
            .context("archiving the neutral role in the test DB")?;
        Ok(s)
    }

    /// A migrated in-memory DB EXACTLY as a brand-new install boots — the
    /// neutral role, the pending preset offer, nothing else. For tests about
    /// fresh-install semantics; everything else wants [`Self::memory`].
    pub async fn memory_bare() -> Result<Self> {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")?
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await?;
        MIGRATOR
            .run(&pool)
            .await
            .context("running sqlite migrations")?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Internal: parameterized 4-way search over cl_index / cl_folders.
    /// `path_column` is the column name varying between tables
    /// (`file_path` for cl_index, `folder_path` for cl_folders). Both
    /// `table` and `path_column` are caller-controlled const strings —
    /// no user input, no injection surface.
    async fn cl_search_table<T>(
        &self,
        table: &str,
        path_column: &str,
        project_id: Option<&str>,
        query: Option<&str>,
        agent_only: bool,
    ) -> Result<Vec<T>>
    where
        T: for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin,
    {
        // `table` and `path_column` are interpolated into the SQL below (sqlx
        // can't bind identifiers), so they MUST be compile-time constants, never
        // user input. Both call sites pass literals; this guard trips a debug
        // build if a future caller forgets. The search term IS bound — see
        // `.bind(&q)` — so it carries no injection risk.
        debug_assert!(
            matches!(table, "cl_index" | "cl_folders"),
            "cl_search_table: non-constant table name {table:?} — identifiers must not be dynamic"
        );
        // `agent_only` filters user-hidden files; only cl_index carries the flag.
        debug_assert!(
            !agent_only || table == "cl_index",
            "agent_only visibility filter is a cl_index concept"
        );
        // Splice shapes for the visibility filter (const strings, no input):
        // after a `WHERE x = ?` clause vs prefixing a bare OR-group.
        let vis_and = if agent_only { " AND agent_visible = 1" } else { "" };
        let vis_pre = if agent_only { "agent_visible = 1 AND " } else { "" };
        let vis_where = if agent_only { " WHERE agent_visible = 1" } else { "" };
        let like = query.map(like_pattern);
        let columns = if table == "cl_index" {
            cl_index_columns()
        } else {
            cl_columns(path_column)
        };
        let select = format!("SELECT {columns} FROM {table}");
        let rows: Vec<T> = match (project_id, like) {
            (Some(pid), Some(q)) => sqlx::query_as::<_, T>(&format!(
                "{select} WHERE project_id = ?{vis_and} AND ( \
                    LOWER({path_column}) LIKE ? ESCAPE '\\' \
                    OR LOWER(description) LIKE ? ESCAPE '\\' \
                    OR LOWER(IFNULL(tags, '')) LIKE ? ESCAPE '\\') \
                 ORDER BY updated_at DESC"
            ))
            .bind(pid)
            .bind(&q)
            .bind(&q)
            .bind(&q)
            .fetch_all(&self.pool)
            .await?,
            (Some(pid), None) => sqlx::query_as::<_, T>(&format!(
                "{select} WHERE project_id = ?{vis_and} ORDER BY updated_at DESC"
            ))
            .bind(pid)
            .fetch_all(&self.pool)
            .await?,
            (None, Some(q)) => sqlx::query_as::<_, T>(&format!(
                "{select} WHERE {vis_pre}(LOWER({path_column}) LIKE ? ESCAPE '\\' \
                    OR LOWER(description) LIKE ? ESCAPE '\\' \
                    OR LOWER(IFNULL(tags, '')) LIKE ? ESCAPE '\\') \
                 ORDER BY updated_at DESC"
            ))
            .bind(&q)
            .bind(&q)
            .bind(&q)
            .fetch_all(&self.pool)
            .await?,
            (None, None) => sqlx::query_as::<_, T>(&format!(
                "{select}{vis_where} ORDER BY updated_at DESC"
            ))
            .fetch_all(&self.pool)
            .await?,
        };
        Ok(rows)
    }
}

/// Column projection for `cl_folders` reads (and the shared base of
/// `cl_index_columns`). The path column differs per table (`file_path` vs
/// `folder_path`), so every folder/file read builds from this and they can't
/// drift. `path_column` is a caller-controlled const, never user input.
fn cl_columns(path_column: &str) -> String {
    format!("id, project_id, {path_column}, description, tags, created_at, updated_at")
}

/// Column projection for `cl_index` reads — the shared list plus
/// `agent_visible`, which only cl_index carries (selecting it from cl_folders
/// would be a SQL error, hence the per-table split in `cl_search_table`).
fn cl_index_columns() -> String {
    format!("{}, agent_visible", cl_columns("file_path"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The applied rows, in version order, as `(version, checksum)`.
    async fn stamps(pool: &SqlitePool) -> Vec<(i64, Vec<u8>)> {
        sqlx::query_as("SELECT version, checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await
            .unwrap()
    }

    /// What every row holds after THIS build applied it: the LF digests.
    fn embedded_stamps() -> Vec<(i64, Vec<u8>)> {
        MIGRATOR
            .iter()
            .map(|m| (m.version, m.checksum.to_vec()))
            .collect()
    }

    /// Re-stamp every row the way a build from a CRLF checkout stamped it —
    /// the state of every Windows database written by bot-hq ≤ 1.0.0.
    async fn stamp_crlf(pool: &SqlitePool) {
        for m in MIGRATOR.iter() {
            sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = ?")
                .bind(crlf_checksum(&m.sql))
                .bind(m.version)
                .execute(pool)
                .await
                .unwrap();
        }
    }

    /// A bare pool on `path` that runs NO migrations and no repair — to look
    /// at a file after `Storage::open` has refused it.
    async fn bare_pool(path: &Path) -> SqlitePool {
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(SqliteConnectOptions::new().filename(path))
            .await
            .unwrap()
    }

    /// (a) The normalised migrator: no CR anywhere, every checksum is the
    /// sha384 of its (LF) sql — the digest sqlx itself uses, which the repair
    /// relies on — and it is the same set of migrations as the raw embedded
    /// one, differing in nothing but line endings.
    #[test]
    fn migration_checksum_migrator_is_lf_normalised_and_sha384_of_its_sql() {
        let raw: Vec<i64> = EMBEDDED_MIGRATIONS.iter().map(|m| m.version).collect();
        let lf: Vec<i64> = MIGRATOR.iter().map(|m| m.version).collect();
        assert_eq!(lf, raw, "normalising must not add, drop or reorder a migration");
        assert!(!lf.is_empty(), "the embedded set is empty");
        // The Migrator-level flags ride along unchanged too.
        assert_eq!(MIGRATOR.ignore_missing, EMBEDDED_MIGRATIONS.ignore_missing);
        assert_eq!(MIGRATOR.locking, EMBEDDED_MIGRATIONS.locking);
        assert_eq!(MIGRATOR.no_tx, EMBEDDED_MIGRATIONS.no_tx);
        for (r, m) in EMBEDDED_MIGRATIONS.iter().zip(MIGRATOR.iter()) {
            assert!(
                !m.sql.contains('\r'),
                "migration {} still carries a carriage return",
                m.version
            );
            assert_eq!(
                &*m.checksum,
                Sha384::digest(m.sql.as_bytes()).as_slice(),
                "migration {}: checksum is not sha384(sql)",
                m.version
            );
            assert_eq!(m.sql, r.sql.replace("\r\n", "\n"), "migration {} text", m.version);
            assert_eq!(m.description, r.description, "migration {} description", m.version);
            assert_eq!(m.migration_type, r.migration_type, "migration {} type", m.version);
            assert_eq!(m.no_tx, r.no_tx, "migration {} no_tx", m.version);
            // The two digests really are different — the whole reason the
            // 1.0.1 build refused the 1.0.0 database. Only where there is a
            // newline to convert: a one-line, newline-free migration has ONE
            // digest under both endings, and the repair's `stored == lf`
            // short-circuit already handles it.
            if m.sql.contains('\n') {
                assert_ne!(
                    crlf_checksum(&m.sql),
                    m.checksum.to_vec(),
                    "migration {}: CRLF and LF digests coincide",
                    m.version
                );
            }
        }
    }

    /// (b) The field failure, exactly: a database every Windows build through
    /// 1.0.0 stamped with CRLF digests opens under this build. First open
    /// stamps LF; every row is then rewritten to the CRLF digest; the second
    /// `Storage::open` — the 1.0.1 launch that used to die before any window
    /// — succeeds and leaves the LF digests behind. Then the count.
    #[tokio::test]
    async fn migration_checksum_repair_recovers_a_db_stamped_by_a_crlf_build() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bot-hq.db");
        let s = Storage::open(&path).await.unwrap();
        assert_eq!(stamps(s.pool()).await, embedded_stamps(), "a fresh install stamps LF");
        stamp_crlf(s.pool()).await;
        assert_ne!(stamps(s.pool()).await, embedded_stamps(), "the fixture did not re-stamp");
        s.pool().close().await;

        let s = Storage::open(&path)
            .await
            .expect("a database stamped by a CRLF-checkout build must open");
        assert_eq!(
            stamps(s.pool()).await,
            embedded_stamps(),
            "every row must now hold the LF digest"
        );

        // The count: one per migration, and nothing to do the second time.
        stamp_crlf(s.pool()).await;
        let repaired = repair_crlf_migration_checksums(s.pool()).await.unwrap();
        assert_eq!(repaired, MIGRATOR.iter().count());
        assert_eq!(stamps(s.pool()).await, embedded_stamps());
        assert_eq!(repair_crlf_migration_checksums(s.pool()).await.unwrap(), 0);
        s.pool().close().await;
    }

    /// (c) A row holding neither the LF nor the CRLF digest is a migration
    /// that really was modified after it was applied: the repair leaves it
    /// alone and the migrator still refuses the database, with sqlx's own
    /// words. The other rows, which WERE CRLF-stamped, are still brought
    /// forward — the refusal is the migrator's, not the repair aborting.
    #[tokio::test]
    async fn migration_checksum_repair_leaves_a_genuinely_modified_migration_to_fail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bot-hq.db");
        let s = Storage::open(&path).await.unwrap();
        stamp_crlf(s.pool()).await;
        let garbage = vec![0xEE_u8; 48];
        sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = 1")
            .bind(&garbage)
            .execute(s.pool())
            .await
            .unwrap();
        s.pool().close().await;

        let err = match Storage::open(&path).await {
            Ok(_) => panic!("opened a database whose migration 1 no longer matches"),
            Err(e) => format!("{e:#}"),
        };
        assert!(
            err.contains("migration 1 was previously applied but has been modified"),
            "expected sqlx's mismatch error in the chain, got: {err}"
        );

        let pool = bare_pool(&path).await;
        let rows = stamps(&pool).await;
        assert_eq!(rows[0], (1, garbage), "the modified row must be untouched");
        assert_eq!(rows[1..], embedded_stamps()[1..], "the CRLF rows were still repaired");
        pool.close().await;
    }

    /// (d) A database already stamped LF: nothing to repair, and the rows —
    /// every column, not just the checksum — are exactly as they were.
    #[tokio::test]
    async fn migration_checksum_repair_is_a_noop_on_lf_stamps() {
        type Row = (i64, String, String, i64, Vec<u8>, i64);
        async fn rows(pool: &SqlitePool) -> Vec<Row> {
            sqlx::query_as(
                "SELECT version, description, CAST(installed_on AS TEXT), \
                 CAST(success AS INTEGER), checksum, execution_time \
                 FROM _sqlx_migrations ORDER BY version",
            )
            .fetch_all(pool)
            .await
            .unwrap()
        }
        let s = Storage::memory_bare().await.unwrap();
        let before = rows(s.pool()).await;
        assert_eq!(
            before.iter().map(|r| (r.0, r.4.clone())).collect::<Vec<_>>(),
            embedded_stamps()
        );
        assert_eq!(repair_crlf_migration_checksums(s.pool()).await.unwrap(), 0);
        assert_eq!(rows(s.pool()).await, before);
    }

    /// (e) A brand-new database has no `_sqlx_migrations` yet: the repair
    /// reports nothing and leaves creating the table to the migrator.
    #[tokio::test]
    async fn migration_checksum_repair_does_not_create_the_table_on_a_fresh_db() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(SqliteConnectOptions::from_str("sqlite::memory:").unwrap())
            .await
            .unwrap();
        assert_eq!(repair_crlf_migration_checksums(&pool).await.unwrap(), 0);
        let tables: Vec<String> =
            sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table'")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert!(tables.is_empty(), "the repair created a table: {tables:?}");
    }
}
