-- declare_working is RETIRED (2026-08-15). The user: "Working state is moot
-- now, in this version" — in a turn ring, working IS holding a turn (the busy
-- map shows it), and every stop is a HALT whose recap is the state. The tool,
-- the badge, and the watchdog suppression flag are gone from the binary;
-- this scrubs the grant from role capability sets so the seeded rows match
-- the sixteen capabilities that exist. Capability::parse already drops
-- unknown slugs, so this is hygiene, not a behavior change — same shape as
-- 0048's route_gated_command cleanup.
--
-- String surgery over json_remove on purpose: the slug appears in exactly
-- one JSON shape (a quoted array element), and covering the three comma
-- positions is total for that shape.
UPDATE roles SET capabilities = REPLACE(capabilities, '"declare_working",', '')
  WHERE capabilities LIKE '%"declare_working",%';
UPDATE roles SET capabilities = REPLACE(capabilities, ',"declare_working"', '')
  WHERE capabilities LIKE '%,"declare_working"%';
UPDATE roles SET capabilities = REPLACE(capabilities, '"declare_working"', '')
  WHERE capabilities LIKE '%"declare_working"%';
