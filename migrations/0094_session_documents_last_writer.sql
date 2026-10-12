-- 0094: who last changed a session document — a participant's slug, or
-- 'user' for a save in the doc's tab (feedback #102). A `session_doc_write`
-- replace by a participant that has not read the current version since
-- someone ELSE changed it is refused, so an agent's rewrite can no longer
-- quietly bury the answers the user typed into the doc (s-079dec53,
-- `decisions-needed`: the user's twelve answers vanished into the archive).
-- NULL on rows written before this column existed: unknown, never refused.
ALTER TABLE session_documents ADD COLUMN last_writer TEXT;
