-- Tag session documents with an IPAV phase so the session view's document
-- tabs (I/P/A/V) can filter by phase, and agents can retrieve prior-phase
-- context via session_doc_search(phase=...). Nullable: existing rows keep
-- NULL and won't surface in any tab or phase-filtered search.

ALTER TABLE session_documents ADD COLUMN phase TEXT;

CREATE INDEX IF NOT EXISTS session_documents_phase_idx
    ON session_documents(session_id, phase);
