-- 0099: a session document's KIND (feedback #89, the user's first slice):
-- 'user_action' marks a doc the user must act on under their own name — a
-- draft to post, an entry to make, a command only they can run. Such docs
-- stay in the cross-session "Your action items" list after the session
-- closes, until the user dismisses them (`dismissed_at`). NULL kind = an
-- ordinary doc. No review step in this slice.
ALTER TABLE session_documents ADD COLUMN kind TEXT;
ALTER TABLE session_documents ADD COLUMN dismissed_at TEXT;
