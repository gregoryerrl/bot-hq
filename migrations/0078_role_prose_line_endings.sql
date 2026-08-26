-- 0078_role_prose_line_endings.sql — role prose written CRLF comes back to LF.
--
-- Every Windows build through 1.0.0 came off a CRLF checkout, and the role
-- prose is a string literal INSIDE the reseed migrations (0046 … 0075), so
-- those builds wrote `roles.description_prompt` with CRLF line endings: on the
-- reporting machine hands was 11918 bytes / 72 CR against the 11846-byte LF
-- seed, eyes 14106 / 90 CR against 14016 — the seed with `\n` → `\r\n`, nothing
-- else. The shipped default the Roles tab compares against
-- (`get_role_default_prose` → the `PRESET_*_ROLE` constants, LF on every build
-- because rustc normalises source literals) therefore never matched, so an
-- untouched pair showed "Differs from the shipped default" with a diff of every
-- line, and a future LF-embedded reseed's byte-exact guard would have skipped
-- the rows. A migration rather than boot-time code: from this build on the
-- migrator embeds LF whatever the checkout, so no new CRLF prose can appear —
-- this is a one-shot over data only the earlier builds wrote.
--
-- Touches only rows that hold a CRLF (a second run matches nothing).
-- `updated_at` is deliberately NOT bumped: the text is unchanged, this is not
-- a user edit. `session_participants.prompt` is left alone — it is the
-- historical spawn-time record, not the editable role.
UPDATE roles
   SET description_prompt = REPLACE(description_prompt, char(13) || char(10), char(10))
 WHERE instr(description_prompt, char(13) || char(10)) > 0;
