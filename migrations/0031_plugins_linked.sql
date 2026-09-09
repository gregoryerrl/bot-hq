-- Linked (dev-mode) installs: serve straight from the user's source
-- directory instead of a copied bundle. When linked=1, dir_path IS the
-- source directory (the serve root) and uninstall must never delete it —
-- it is the user's repo, not bot-hq's data.
ALTER TABLE plugins ADD COLUMN linked INTEGER NOT NULL DEFAULT 0;
