-- 1.0.0 Batch 4 (the user, 2026-08-24): "I want a clean default for
-- everything, for example: a default role (no instruction prompt). HANDS and
-- EYES are my personal roles, other users will have their own configs."
--
-- A FRESH install stops being born with the author's two roles and ~26 KB of
-- his workflow doctrine (0044 seeds them, 0046→0068 load the prose). It gets
-- ONE neutral role — 'agent', every capability, NO prose — plus a one-time
-- offer flag the Roles tab reads to show "install the example pair" (install
-- or decline once; an ABSENT flag means no offer, so a used install never
-- sees it).
--
-- EVERY statement is guarded by the fresh-DB discriminator
-- `(SELECT COUNT(*) FROM sessions) = 0` — true on a brand-new DB by
-- construction, false on any DB that ever ran a session — so an UPGRADING
-- install (the author's included) is untouched: his hands/eyes rows, prose
-- edits and all, survive byte-for-byte. The DELETE carries a second,
-- independent guard on participant references: even if the discriminator were
-- broken by a future edit, a role any session ever used cannot be deleted.
DELETE FROM roles
 WHERE slug IN ('hands', 'eyes')
   AND (SELECT COUNT(*) FROM sessions) = 0
   AND NOT EXISTS (SELECT 1 FROM session_participants sp WHERE sp.role_id = roles.id);

INSERT INTO roles
    (slug, display_name, description_prompt, capabilities, participation_mode, builtin)
SELECT 'agent', 'Agent', NULL,
       json('["read_channel","post_channel","ask_user","park_approval",
              "supersede_question","halt","close_session","file_finding",
              "approve_finding","disposition_finding","override_reviewer_block",
              "edit_files","run_bash","gated_bash","run_terminal",
              "write_context_library"]'),
       'active', 0
 WHERE (SELECT COUNT(*) FROM sessions) = 0
   AND NOT EXISTS (SELECT 1 FROM roles WHERE slug = 'agent');

INSERT INTO app_settings (key, value)
SELECT 'role_preset_offer', 'pending'
 WHERE (SELECT COUNT(*) FROM sessions) = 0
   AND NOT EXISTS (SELECT 1 FROM app_settings WHERE key = 'role_preset_offer');
