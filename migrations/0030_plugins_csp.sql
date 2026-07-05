-- Consent-frozen CSP grant per plugin: the canonical serialization of the
-- csp_extra_origins the USER approved at install time. Serving reads ONLY
-- this column (never the manifest on disk or the stored manifest_json), so
-- a manifest stored by a pre-CSP host can never activate origins after a
-- host upgrade — NULL means the strict default CSP, forever, until a
-- re-install re-consents.
ALTER TABLE plugins ADD COLUMN csp_json TEXT;
