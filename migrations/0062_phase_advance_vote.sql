-- The phase-advance vote: every active participant must agree before the phase
-- moves, so each one genuinely gets a turn at the boundary.
--
-- The user's design, from session s-dbc0e856 (2026-08-16), after observing a
-- full IPAV pass in which the reviewer was never dealt a turn:
--
--   "let's force that each phase advance must be voted of all active
--    participants. So each participant will genuinely have a turn. After
--    investigation, HANDS will vote to advance_phase, but it doesn't advance the
--    phase yet, instead will rotate the turn, EYES is next, if EYES found issues
--    with investigation, then EYES will not vote to advance_phase, EYES will
--    instead rotate the turn and drop their findings... So they literally must
--    converge and smoke the issues before phase advance."
--
-- ## Why this is not a column on session_participants
--
-- `done_vote` is one, and reusing it LIVELOCKS. `sequencer.rs` clears every
-- done vote on `TurnEnding::Spoke` — substantive output resets the tally — and
-- this feature exists to produce speech (findings) between votes. Every finding
-- would wipe the votes preceding it and the session could never converge.
--
-- So a phase vote is a ROW that carries what it was cast about, and it stops
-- counting when that thing changes rather than when anyone talks.
--
-- ## The key, and why it needs both halves
--
-- `(participant_id, target_phase, artifact_fingerprint, phase_epoch)`.
--
-- **`artifact_fingerprint`** closes the speech axis: a vote is about a specific
-- state of the work, so talking never invalidates it and changing the work
-- always does. It digests ALL of the session's phase documents — count, latest
-- `updated_at`, total body length — rather than the current phase's alone. That
-- keeps it answerable from storage (the live phase is in-memory `AppState` and
-- unreachable where the vote is cast) and is the stricter rule anyway: editing
-- the Investigate document while voting to leave Plan invalidates too, which is
-- correct, because the votes were cast on a body of work that has since moved.
--
-- A session with no documents has a stable digest of its empty state, so a phase
-- that legitimately produces no document can still be voted through. An earlier
-- draft of this comment specified a sentinel that no vote could match, to make
-- empty work unvotable; that was rejected before implementation. The guard
-- against advancing on nothing is the REVIEWER'S vote, which is a judgement about
-- whether the work is done — a sentinel would instead have made a whole class of
-- phase permanently unadvanceable.
--
-- **`phase_epoch`** closes the TIME axis, and it is the half the first design
-- missed — caught in review. Phases run backward. A vote cast in Plan survives
-- Plan -> Investigate -> Plan whenever the artifact returns to the same content
-- (a revert, or simply no edit), and would then count toward a tally about a
-- different conversation. A fingerprint carries content identity, not time. The
-- epoch is monotonic per session and bumps on every transition, so a vote from
-- before a round trip can never match after it.
--
-- The epoch IS a reset trigger by another name, and the reason that is
-- acceptable here is that it has exactly ONE production call site — `AppState`'s
-- phase writer — which is a different risk profile from "reset on the right set
-- of events", the predicate shape this codebase has shipped wrong five times.
--
-- ## Deletion, not accumulation
--
-- A vote row is only ever valid for one (target, fingerprint, epoch) triple, so
-- stale rows are deleted rather than kept: the tally is a COUNT of live rows and
-- a leftover row is a vote nobody cast for the question being asked. The
-- `ON DELETE CASCADE` means a participant leaving takes its votes with it, which
-- is the same rule the electorate uses — you cannot hold a vote in a roster you
-- are not in.
CREATE TABLE IF NOT EXISTS phase_votes (
    session_id           TEXT    NOT NULL,
    participant_id       INTEGER NOT NULL REFERENCES session_participants(id) ON DELETE CASCADE,
    target_phase         TEXT    NOT NULL,
    artifact_fingerprint TEXT    NOT NULL,
    phase_epoch          INTEGER NOT NULL,
    created_at           TEXT    NOT NULL,
    PRIMARY KEY (participant_id, target_phase, artifact_fingerprint, phase_epoch)
);

CREATE INDEX IF NOT EXISTS idx_phase_votes_session
    ON phase_votes (session_id, target_phase, artifact_fingerprint, phase_epoch);

-- Monotonic, bumped on every phase transition. Nullable-free with a default so
-- every existing session starts at 0 and no INSERT needs updating — the same
-- shape 0058 used, and for the same reason: a NOT NULL column with no default is
-- what stopped the app booting in 0044.
ALTER TABLE sessions ADD COLUMN phase_epoch INTEGER NOT NULL DEFAULT 0;
