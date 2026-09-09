-- 1.0.1: one-time starter offers for the Tool Gate keyword list and the
-- general policy (the user, 2026-08-26: "also seed the other defaults — but
-- only the basic ones; they can configure as they go"). Mirrors 0072's
-- role_preset_offer exactly: the Settings cards render only while the value
-- is the literal 'pending'; the resolve commands stamp 'installed' /
-- 'declined'; an ABSENT key means no offer.
--
-- Two keys, not one, so gates and policy can be adopted independently.
--
-- Fresh-DB discriminator as in 0072: `(SELECT COUNT(*) FROM sessions) = 0`.
-- Installs that predate this migration but never wrote a config file are
-- covered by the boot backfill in main.rs (key absent AND the file absent →
-- 'pending'), so an already-configured install never sees an offer.

INSERT INTO app_settings (key, value)
SELECT 'gate_preset_offer', 'pending'
 WHERE (SELECT COUNT(*) FROM sessions) = 0
   AND NOT EXISTS (SELECT 1 FROM app_settings WHERE key = 'gate_preset_offer');

INSERT INTO app_settings (key, value)
SELECT 'policy_preset_offer', 'pending'
 WHERE (SELECT COUNT(*) FROM sessions) = 0
   AND NOT EXISTS (SELECT 1 FROM app_settings WHERE key = 'policy_preset_offer');
