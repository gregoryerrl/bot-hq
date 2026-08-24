-- 1.0.0 Batch 5 (config-sweep must-change M3): the 0016 seed tops out at
-- claude-opus-4-8 with every context_window NULL — a fresh install's model
-- picker is a list of two-generation-old ids and its context meter has no
-- denominator. FRESH DBs (COUNT(sessions)=0) shed the stale seed and get the
-- current generation with confirmed windows. USED DBs are CURATED — the
-- author deleted 8 of the 13 seeds deliberately — so on them this migration
-- inserts NOTHING (the fresh-only guard covers every insert; the
-- NOT-EXISTS-on-model_name guard is belt-and-suspenders against a collision
-- with a curated row, e.g. the live registry's repurposed claude-opus-5 row).
--
-- The ONE thing this migration does on a used DB is repair the context_window
-- data gap where the value is KNOWN: rows whose model_name exactly matches a
-- current Anthropic model get that model's published window IF the column is
-- NULL. 0037's contract ("a visible gap, never a guessed number") holds — these
-- are published numbers keyed on exact names, and an unrecognized model_name
-- stays NULL.
DELETE FROM models
 WHERE (SELECT COUNT(*) FROM sessions) = 0
   AND id IN ('claude-opus-4-8','claude-opus-4-7','claude-opus-4-6',
              'claude-opus-4-5','claude-opus-4-1','claude-opus-4',
              'claude-sonnet-4-6','claude-sonnet-4-5','claude-sonnet-4',
              'claude-sonnet-3-7','claude-haiku-4-5','claude-haiku-4',
              'claude-haiku-3-5');

INSERT INTO models (id, display_name, provider, model_name, context_window)
SELECT 'claude-opus-5', 'Claude Opus 5', 'anthropic', 'claude-opus-5', 1000000
 WHERE (SELECT COUNT(*) FROM sessions) = 0
   AND NOT EXISTS (SELECT 1 FROM models WHERE model_name = 'claude-opus-5');

INSERT INTO models (id, display_name, provider, model_name, context_window)
SELECT 'claude-sonnet-5', 'Claude Sonnet 5', 'anthropic', 'claude-sonnet-5', 1000000
 WHERE (SELECT COUNT(*) FROM sessions) = 0
   AND NOT EXISTS (SELECT 1 FROM models WHERE model_name = 'claude-sonnet-5');

INSERT INTO models (id, display_name, provider, model_name, context_window)
SELECT 'claude-fable-5', 'Claude Fable 5', 'anthropic', 'claude-fable-5', 1000000
 WHERE (SELECT COUNT(*) FROM sessions) = 0
   AND NOT EXISTS (SELECT 1 FROM models WHERE model_name = 'claude-fable-5');

INSERT INTO models (id, display_name, provider, model_name, context_window)
SELECT 'claude-haiku-4-5', 'Claude Haiku 4.5', 'anthropic', 'claude-haiku-4-5', 200000
 WHERE (SELECT COUNT(*) FROM sessions) = 0
   AND NOT EXISTS (SELECT 1 FROM models WHERE model_name = 'claude-haiku-4-5');

UPDATE models SET context_window = 1000000
 WHERE context_window IS NULL
   AND model_name IN ('claude-opus-5','claude-sonnet-5','claude-fable-5',
                      'claude-opus-4-8','claude-opus-4-7','claude-opus-4-6',
                      'claude-sonnet-4-6');

UPDATE models SET context_window = 200000
 WHERE context_window IS NULL
   AND model_name = 'claude-haiku-4-5';
