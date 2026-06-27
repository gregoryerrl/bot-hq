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
        for atom in atoms {
            sqlx::query(
                "INSERT INTO cl_atoms(project_id, file_path, heading_path, body, mtime, body_hash) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(project_id)
            .bind(file_path)
            .bind(&atom.heading_path)
            .bind(&atom.body)
            .bind(mtime)
            .bind(atom_body_hash(&atom.body))
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

#[cfg(test)]
mod tests {
    use super::{atom_body_hash, Atom};
    use crate::storage::Storage;

    fn atom(heading: &str, body: &str) -> Atom {
        Atom { heading_path: heading.into(), body: body.into() }
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

    #[test]
    fn body_hash_is_deterministic_and_64_hex() {
        // SHA-256 is stable across calls/processes (unlike DefaultHasher).
        assert_eq!(atom_body_hash("the same body"), atom_body_hash("the same body"));
        assert_ne!(atom_body_hash("body one"), atom_body_hash("body two"));
        let h = atom_body_hash("");
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
