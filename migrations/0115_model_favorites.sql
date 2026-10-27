-- A UI preference, separate from the model's endpoint/runtime configuration.
ALTER TABLE models ADD COLUMN favorite INTEGER NOT NULL DEFAULT 0 CHECK (favorite IN (0, 1));
