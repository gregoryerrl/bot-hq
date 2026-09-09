-- Record the ORIGINAL install source for copy-mode plugins (local
-- directory, or the manifest URL for URL installs). Linked rows leave it
-- NULL: dir_path IS the source. Powers "Update from source" on copy-mode
-- cards and the Reinstall dialog's source pre-fill. Rows installed before
-- this migration stay NULL — those plugins update via Reinstall.
ALTER TABLE plugins ADD COLUMN source_path TEXT;
