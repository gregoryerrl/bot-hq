//! `cl_atoms`: the FTS5 atom index backing queryable CL retrieval (Phase 3).
//!
//! An *atom* is one heading-delimited section of a CL file. The table is a
//! standalone FTS5 virtual table (`migrations/0024_cl_atoms.sql`): `heading_path`
//! and `body` are tokenized / BM25-searchable, the rest is UNINDEXED metadata.
//! Like `cl_index` it is a derived, disposable layer — `cl_rescan` repopulates it
//! from disk (`split_into_atoms` does the splitting). The ranked read side
//! (`cl_retrieve`) lands in Stage 2; this module owns the write side.

use super::*;

/// A heading-delimited section of a CL file, pre-insertion. `heading_path` is the
/// "H1 > H2" breadcrumb ("(intro)" for preamble before the first heading); `body`
/// is the section text. Produced by `signaling::bridge::util::split_into_atoms`,
/// consumed by [`Storage::replace_atoms_for_file`] (which computes the stored
/// SHA-256 `body_hash`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Atom {
    pub heading_path: String,
    pub body: String,
    /// SHA-256 over the repo source files this atom cites (e.g. `src/foo.rs`), as
    /// they were when the atom was indexed — `None` when the atom cites no
    /// existing source. Computed by the bridge (which has repo access) before
    /// insertion; retrieval recomputes it to flag possibly-stale atoms. See
    /// `signaling::bridge::cl_refs`.
    pub code_hash: Option<String>,
}

/// One atom returned by [`Storage::cl_retrieve`]: which file/section it came from
/// plus the body to inline. Distinct from [`Atom`] (the pre-insert form) by
/// carrying `file_path` so callers can cite the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetrievedAtom {
    pub file_path: String,
    pub heading_path: String,
    pub body: String,
    /// Stored `code_hash` baseline (see [`Atom::code_hash`]); `None` when the atom
    /// cites no code. Storage returns it; the bridge wrapper uses it to compute
    /// `stale` and callers generally don't read it directly.
    pub code_hash: Option<String>,
    /// Set by the bridge `cl_retrieve` wrapper (NOT storage): the cited code has
    /// drifted since indexing, so the atom may be out of date. Storage always
    /// returns `false`; the MCP layer renders a ⚠ prefix when true.
    pub stale: bool,
}

impl Storage {
    /// Replace ALL atoms for one CL file in a single transaction: delete the
    /// file's existing atoms, then insert the freshly-split set. Called by
    /// `cl_rescan` when a file is added or its body changed on disk. Atomic so a
    /// mid-replace failure can't leave a half-updated file (stale + fresh atoms).
    pub async fn replace_atoms_for_file(
        &self,
        project_id: &str,
        file_path: &str,
        atoms: &[Atom],
        mtime: &str,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM cl_atoms WHERE project_id = ? AND file_path = ?")
            .bind(project_id)
            .bind(file_path)
            .execute(&mut *tx)
            .await?;
        // kind is per-FILE (derived from its path), stamped on every atom so
        // retrieval can pin/decay by kind without re-deriving from the filename.
        let kind = cl_kind_for_path(file_path);
        for atom in atoms {
            sqlx::query(
                "INSERT INTO cl_atoms(project_id, file_path, kind, heading_path, body, mtime, body_hash, code_hash) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(project_id)
            .bind(file_path)
            .bind(kind)
            .bind(&atom.heading_path)
            .bind(&atom.body)
            .bind(mtime)
            .bind(atom_body_hash(&atom.body))
            .bind(atom.code_hash.as_deref())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Delete every atom for a file — the `cl_rescan` orphan branch (file gone on
    /// disk). Returns the row count removed. The derived atom layer follows the
    /// index: when `cl_index` purges a dangling row, its atoms go too.
    pub async fn delete_atoms_for_file(&self, project_id: &str, file_path: &str) -> Result<u64> {
        let res = sqlx::query("DELETE FROM cl_atoms WHERE project_id = ? AND file_path = ?")
            .bind(project_id)
            .bind(file_path)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected())
    }

    /// Count atoms for one file. Used by `cl_rescan` to backfill the derived FTS
    /// layer after migration 0024 when `cl_index` rows predate `cl_atoms`.
    pub async fn count_atoms_for_file(&self, project_id: &str, file_path: &str) -> Result<i64> {
        let n = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cl_atoms WHERE project_id = ? AND file_path = ?",
        )
        .bind(project_id)
        .bind(file_path)
        .fetch_one(&self.pool)
        .await?;
        Ok(n)
    }

    /// Ranked body retrieval over the FTS atom index — the read side of Phase 3.
    /// Returns the atoms whose `heading_path`/`body` best match `query` (FTS5
    /// BM25), scoped to `project_id`, optionally restricted to `paths`, and
    /// accumulated under `budget_tokens` (a coarse ~chars/4 estimate). On a BM25
    /// tie, convention/decision-kind atoms win (pin), then fresher atoms (mtime),
    /// then file_path/heading_path for a deterministic, repeatable order.
    /// The query is sanitized into a safe FTS5 MATCH expression so arbitrary input
    /// can't throw a syntax error; a query with no searchable tokens returns empty.
    pub async fn cl_retrieve(
        &self,
        project_id: &str,
        query: &str,
        paths: Option<&[String]>,
        budget_tokens: i64,
    ) -> Result<Vec<RetrievedAtom>> {
        let match_expr = to_fts5_match(query);
        if match_expr.is_empty() {
            return Ok(Vec::new());
        }
        let path_filter = paths.filter(|p| !p.is_empty());

        // Pin (convention/decision kind) + freshness are tie-breakers AFTER bm25
        // relevance, then file_path/heading_path as a DETERMINISTIC final key so
        // identical queries return identical order (without it, full ties — same
        // bm25, kind, and mtime, e.g. all atoms of one file — are SQLite-ordered
        // and a budget trim could drop different atoms run-to-run). The pinned
        // kinds are constant literals (no injection); the optional path filter
        // binds each path as a parameter. A safety LIMIT caps rows pulled before
        // the Rust-side token-budget trim. rowid is the ultimate key: a section
        // can emit multiple sub-atoms sharing one heading_path, so document order
        // keeps the budget trim deterministic.
        let mut sql = String::from(
            "SELECT file_path, heading_path, body, code_hash FROM cl_atoms \
             WHERE project_id = ? AND cl_atoms MATCH ?",
        );
        if let Some(paths) = path_filter {
            sql.push_str(" AND file_path IN (");
            for i in 0..paths.len() {
                if i > 0 {
                    sql.push_str(", ");
                }
                sql.push('?');
            }
            sql.push(')');
        }
        sql.push_str(
            " ORDER BY bm25(cl_atoms), \
             CASE WHEN kind IN ('convention', 'decision') THEN 0 ELSE 1 END, \
             mtime DESC, file_path, heading_path, rowid LIMIT 128",
        );

        let mut q = sqlx::query_as::<_, (String, String, String, Option<String>)>(&sql)
            .bind(project_id)
            .bind(&match_expr);
        if let Some(paths) = path_filter {
            for p in paths {
                q = q.bind(p);
            }
        }
        let rows = q.fetch_all(&self.pool).await?;

        // Accumulate under the budget; always keep at least the top atom so one
        // oversized atom can't make the whole retrieval return empty.
        let mut out = Vec::new();
        let mut used = 0i64;
        for (file_path, heading_path, body, code_hash) in rows {
            let cost = estimate_tokens(&body);
            if !out.is_empty() && used + cost > budget_tokens {
                break;
            }
            used += cost;
            out.push(RetrievedAtom { file_path, heading_path, body, code_hash, stale: false });
        }
        Ok(out)
    }
}

/// SHA-256 hex digest (64 lowercase hex chars) of an atom body. Deterministic and
/// stable across processes AND Rust releases — mirrors `policy::audit::content_hash`.
/// Stored for future retrieval-time stale-flagging. NOT `DefaultHasher`, whose
/// algorithm is std-documented as unstable across releases (a toolchain bump would
/// re-hash every body and report spurious "changed").
fn atom_body_hash(body: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(body.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Classify a CL file by its project-relative path into a coarse `kind`
/// (convention|decision|policy|issue|idea|handoff|gotcha|note), stamped on every
/// atom of that file so retrieval can pin/decay by kind instead of by hardcoded
/// filenames. Inferred from the canonical filenames + the `plans/` handoff
/// convention (CL-v2 assessment §4: "kind inferred from source file"). `note` is
/// the catch-all for anything off the canonical set — an extension of the brief's
/// taxonomy, but better than NULL. Only the canonical ROOT path matches (a nested
/// `sub/conventions.md` is `note`), so the retrieval pin can't be fooled by a
/// same-named file deeper in the tree.
fn cl_kind_for_path(file_path: &str) -> &'static str {
    if file_path.starts_with("plans/") {
        return "handoff";
    }
    match file_path {
        "conventions.md" => "convention",
        "decisions.md" => "decision",
        "policy.yaml" => "policy",
        "issues.md" => "issue",
        "ideas.md" => "idea",
        "notes.md" => "gotcha",
        _ => "note",
    }
}

/// Turn an arbitrary user query into a safe FTS5 MATCH expression: extract
/// alphanumeric tokens, quote each one INDIVIDUALLY (which neutralizes FTS5
/// operators like AND/OR/NOT/NEAR and metacharacters like `* " -`), and OR them
/// for recall — BM25 then ranks by how well each atom matches. Empty when the
/// query has no alphanumeric tokens (the caller treats that as "no results").
///
/// Quoting per-token does NOT disable stemming: FTS5 tokenizes the contents of a
/// quoted string, so `porter` still applies (migrate / migration / migrations
/// share a stem — guarded by `cl_retrieve_stems_across_inflections`). Quoting each
/// token SEPARATELY (vs quoting the whole query as one phrase) also avoids
/// imposing phrase-adjacency, so word order doesn't constrain matches.
fn to_fts5_match(query: &str) -> String {
    query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// Rough token estimate for budgeting: ~4 chars per token. Coarse and English-
/// biased — it UNDERSHOOTS for CJK/multibyte text (1 char ≉ 0.25 tokens) — but it
/// is only a budget guardrail, not a billing figure.
pub(crate) fn estimate_tokens(text: &str) -> i64 {
    (text.chars().count() as i64 + 3) / 4
}

#[cfg(test)]
mod tests {
    use super::{atom_body_hash, Atom};
    use crate::storage::Storage;

    fn atom(heading: &str, body: &str) -> Atom {
        Atom { heading_path: heading.into(), body: body.into(), code_hash: None }
    }

    /// Stage-0 de-risk gate (kept): proves FTS5 is compiled into the bundled sqlite
    /// and that standalone-table INSERT, MATCH, and BM25 ranking all work. If FTS5
    /// were absent, migration 0024 (`CREATE VIRTUAL TABLE … USING fts5`) would have
    /// failed inside `Storage::memory()` and we'd never reach the asserts.
    #[tokio::test]
    async fn fts5_atoms_match_and_bm25_rank() {
        let s = Storage::memory().await.unwrap();
        s.replace_atoms_for_file(
            "bot-hq",
            "notes.md",
            &[
                atom("Gotchas", "the applied sqlx migration is immutable once shipped"),
                atom("Setup", "run cargo build then start the desktop app"),
            ],
            "2026-06-27T00:00:00+00:00",
        )
        .await
        .unwrap();

        // MATCH on a term unique to the first atom; BM25-ordered (lower = better).
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT heading_path FROM cl_atoms WHERE cl_atoms MATCH ? ORDER BY bm25(cl_atoms)",
        )
        .bind("migration")
        .fetch_all(s.pool())
        .await
        .unwrap();
        assert_eq!(rows.len(), 1, "MATCH 'migration' should hit exactly the Gotchas atom");
        assert_eq!(rows[0].0, "Gotchas");

        // A term in neither atom returns nothing (sanity).
        let none: Vec<(String,)> =
            sqlx::query_as("SELECT heading_path FROM cl_atoms WHERE cl_atoms MATCH ?")
                .bind("kubernetes")
                .fetch_all(s.pool())
                .await
                .unwrap();
        assert!(none.is_empty(), "MATCH of an absent term must return no rows");
    }

    #[tokio::test]
    async fn replace_atoms_is_idempotent_and_clears_old() {
        let s = Storage::memory().await.unwrap();
        s.replace_atoms_for_file("p", "notes.md", &[atom("A", "alpha"), atom("B", "beta")], "t1")
            .await
            .unwrap();
        // Re-replace with a different set → old atoms gone, only the new set remains.
        s.replace_atoms_for_file("p", "notes.md", &[atom("C", "gamma")], "t2")
            .await
            .unwrap();
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT heading_path FROM cl_atoms WHERE project_id='p' AND file_path='notes.md' ORDER BY heading_path",
        )
        .fetch_all(s.pool())
        .await
        .unwrap();
        assert_eq!(rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(), vec!["C"]);
    }

    #[tokio::test]
    async fn delete_atoms_removes_only_that_file() {
        let s = Storage::memory().await.unwrap();
        s.replace_atoms_for_file("p", "a.md", &[atom("x", "xx")], "t").await.unwrap();
        s.replace_atoms_for_file("p", "b.md", &[atom("y", "yy")], "t").await.unwrap();
        let removed = s.delete_atoms_for_file("p", "a.md").await.unwrap();
        assert_eq!(removed, 1);
        let remaining: Vec<(String,)> =
            sqlx::query_as("SELECT file_path FROM cl_atoms ORDER BY file_path")
                .fetch_all(s.pool())
                .await
                .unwrap();
        assert_eq!(remaining.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(), vec!["b.md"]);
    }

    #[tokio::test]
    async fn replace_atoms_stamps_kind_from_path() {
        let s = Storage::memory().await.unwrap();
        // kind is derived from the file path (one kind per file), not the section.
        s.replace_atoms_for_file("p", "conventions.md", &[atom("H", "b")], "t").await.unwrap();
        s.replace_atoms_for_file("p", "plans/2026-handoff.md", &[atom("H", "b")], "t").await.unwrap();
        s.replace_atoms_for_file("p", "weird.txt", &[atom("H", "b")], "t").await.unwrap();
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT file_path, kind FROM cl_atoms WHERE project_id='p' ORDER BY file_path",
        )
        .fetch_all(s.pool())
        .await
        .unwrap();
        assert_eq!(
            rows,
            vec![
                ("conventions.md".into(), "convention".into()),
                ("plans/2026-handoff.md".into(), "handoff".into()),
                ("weird.txt".into(), "note".into()),
            ]
        );
    }

    #[test]
    fn cl_kind_for_path_maps_canonical_files() {
        use super::cl_kind_for_path;
        assert_eq!(cl_kind_for_path("conventions.md"), "convention");
        assert_eq!(cl_kind_for_path("decisions.md"), "decision");
        assert_eq!(cl_kind_for_path("policy.yaml"), "policy");
        assert_eq!(cl_kind_for_path("issues.md"), "issue");
        assert_eq!(cl_kind_for_path("ideas.md"), "idea");
        assert_eq!(cl_kind_for_path("notes.md"), "gotcha");
        assert_eq!(cl_kind_for_path("plans/anything.md"), "handoff");
        assert_eq!(cl_kind_for_path("subdir/conventions.md"), "note"); // only the canonical root path pins
        assert_eq!(cl_kind_for_path("whatever.md"), "note");
    }

    #[test]
    fn body_hash_is_deterministic_and_64_hex() {
        // SHA-256 is stable across calls/processes (unlike DefaultHasher).
        assert_eq!(atom_body_hash("the same body"), atom_body_hash("the same body"));
        assert_ne!(atom_body_hash("body one"), atom_body_hash("body two"));
        let h = atom_body_hash("");
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[tokio::test]
    async fn cl_retrieve_ranks_by_relevance_and_scopes_by_project() {
        let s = Storage::memory().await.unwrap();
        s.replace_atoms_for_file(
            "p",
            "notes.md",
            &[
                atom("Migrations", "applied sqlx migrations are immutable"),
                atom("Frontend", "react tailwind vitest"),
            ],
            "t",
        )
        .await
        .unwrap();
        // A different project must not leak into p's results.
        s.replace_atoms_for_file("other", "x.md", &[atom("Migrations", "immutable migration here")], "t")
            .await
            .unwrap();

        let hits = s.cl_retrieve("p", "migration immutable", None, 10_000).await.unwrap();
        assert!(!hits.is_empty());
        assert!(hits.iter().all(|h| h.file_path == "notes.md"), "project-scoped");
        assert_eq!(hits[0].heading_path, "Migrations", "best match ranks first");
    }

    #[tokio::test]
    async fn cl_retrieve_sanitizes_adversarial_queries() {
        let s = Storage::memory().await.unwrap();
        s.replace_atoms_for_file("p", "n.md", &[atom("H", "alpha beta gamma")], "t").await.unwrap();
        // FTS5 operators / metacharacters must not throw — they're quoted literals.
        for q in ["alpha AND", "beta\"", "*", "gamma NEAR/2", "   ", "a OR b", "-x"] {
            s.cl_retrieve("p", q, None, 10_000).await.unwrap();
        }
        assert!(!s.cl_retrieve("p", "alpha", None, 10_000).await.unwrap().is_empty());
        // No alphanumeric tokens → no MATCH run, empty result.
        assert!(s.cl_retrieve("p", "***", None, 10_000).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn cl_retrieve_respects_token_budget() {
        let s = Storage::memory().await.unwrap();
        let big = "lorem ".repeat(20); // 120 chars → ~30 tokens each
        s.replace_atoms_for_file("p", "n.md", &[atom("A", &big), atom("B", &big), atom("C", &big)], "t")
            .await
            .unwrap();
        // Budget for ~one atom → exactly one (we always keep at least the top one).
        assert_eq!(s.cl_retrieve("p", "lorem", None, 30).await.unwrap().len(), 1);
        // Generous budget → all three.
        assert_eq!(s.cl_retrieve("p", "lorem", None, 10_000).await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn cl_retrieve_paths_filter_restricts_files() {
        let s = Storage::memory().await.unwrap();
        s.replace_atoms_for_file("p", "a.md", &[atom("H", "shared keyword")], "t").await.unwrap();
        s.replace_atoms_for_file("p", "b.md", &[atom("H", "shared keyword")], "t").await.unwrap();
        let only = vec!["a.md".to_string()];
        let hits = s.cl_retrieve("p", "keyword", Some(&only), 10_000).await.unwrap();
        assert!(!hits.is_empty());
        assert!(hits.iter().all(|h| h.file_path == "a.md"));
    }

    #[tokio::test]
    async fn cl_retrieve_pins_conventions_on_bm25_tie() {
        let s = Storage::memory().await.unwrap();
        // Identical body → identical BM25; the pin tie-break puts conventions first.
        s.replace_atoms_for_file("p", "notes.md", &[atom("H", "deploy gate keyword")], "t").await.unwrap();
        s.replace_atoms_for_file("p", "conventions.md", &[atom("H", "deploy gate keyword")], "t")
            .await
            .unwrap();
        let hits = s.cl_retrieve("p", "deploy gate keyword", None, 10_000).await.unwrap();
        assert_eq!(hits[0].file_path, "conventions.md", "pinned file wins the bm25 tie");
    }

    #[tokio::test]
    async fn cl_retrieve_tie_order_is_deterministic_by_path() {
        let s = Storage::memory().await.unwrap();
        // Identical body → BM25 tie; identical mtime; neither pinned (kind=note)
        // → the deterministic file_path/heading_path tie-break decides order.
        // Without it SQLite order is unspecified and a budget trim could vary.
        s.replace_atoms_for_file("p", "zeta.md", &[atom("H", "shared body keyword")], "t").await.unwrap();
        s.replace_atoms_for_file("p", "alpha.md", &[atom("H", "shared body keyword")], "t").await.unwrap();
        let hits = s.cl_retrieve("p", "shared body keyword", None, 10_000).await.unwrap();
        let order: Vec<&str> = hits.iter().map(|h| h.file_path.as_str()).collect();
        assert_eq!(order, vec!["alpha.md", "zeta.md"], "full ties order by file_path");
    }

    #[test]
    fn to_fts5_match_quotes_tokens_and_ors() {
        assert_eq!(super::to_fts5_match("alpha AND beta"), "\"alpha\" OR \"AND\" OR \"beta\"");
        assert_eq!(super::to_fts5_match("  *!*  "), "");
        assert_eq!(super::to_fts5_match("one"), "\"one\"");
    }

    #[tokio::test]
    async fn cl_retrieve_stems_across_inflections() {
        let s = Storage::memory().await.unwrap();
        s.replace_atoms_for_file("p", "n.md", &[atom("H", "applied migrations are immutable")], "t")
            .await
            .unwrap();
        // Query inflections the body does NOT contain verbatim. FTS5 tokenizes the
        // contents of a quoted string, so `porter` stemming still applies through
        // to_fts5_match's quoting: migrate/migration/migrations share a stem.
        for q in ["migrate", "migration", "MIGRATING"] {
            assert!(
                !s.cl_retrieve("p", q, None, 10_000).await.unwrap().is_empty(),
                "porter stemming should match {q:?} -> migrations (quoting keeps the tokenizer)"
            );
        }
    }
}
