-- 0092: the user's OWN words in a session, one row per message the user
-- typed, staged or answered in the tray — so that what the user said since a
-- participant's handoff doc was last written can be put back VERBATIM into
-- that participant's context after a compaction (`agents::handoff`, the
-- `compact-user-words` hook).
--
-- A table rather than a mark on `messages`: a user-origin row is not always
-- the user speaking — the resume notice, the close-out prompt, a plugin's
-- send and an approved gate's output all post as origin `user` — so the
-- CALLER that knows whose words they are records them here, and nothing has
-- to re-derive it from the text. `text` is what is put back: the typed
-- message as written; for a tray answer the pick with the question it
-- answered, not the row's whole body (which carries the gate's output).
-- `message_id` ties the entry to its channel row.
CREATE TABLE session_user_words (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id  TEXT    NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    message_id  INTEGER NOT NULL,
    created_at  TEXT    NOT NULL,
    text        TEXT    NOT NULL
);
CREATE INDEX idx_session_user_words_session ON session_user_words(session_id, id);
