-- 0095: where a session document was promoted to in the Context Library, and
-- when — set by `cl_write_file(session_doc: <slug>)`, the byte-for-byte copy
-- (feedback #73): the close recap lists the CUSTOM documents never promoted,
-- so approved content that lives only in a session doc is not archived with
-- the session unnoticed (s-90ac85d7: the user's approved key questions lived
-- only in `key-questions` until an agent copied them by hand at close).
-- `promoted_to` is `<project>/<cl file path>`; NULL = never promoted.
ALTER TABLE session_documents ADD COLUMN promoted_to TEXT;
ALTER TABLE session_documents ADD COLUMN promoted_at TEXT;
