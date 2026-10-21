-- 0103: the trust hash Codex reports for bot-hq's gate hook in a managed
-- Codex home, cached so a spawn can grant it up front.
--
-- Codex runs a file-resident hook only when its trust hash is recorded;
-- bot-hq grants that per spawn (`-c hooks.state={…}`) but must know the hash
-- first, and only `hooks/list` on a running app-server reports it. The first
-- spawn into a home therefore starts, asks, caches the hash here — after the
-- listed entry matched what bot-hq rendered, exactly — and restarts with the
-- grant; every later spawn reads the row and starts once. The Codex version
-- is kept beside it because the hash is Codex's own derivation of the hook
-- definition and may change with the CLI; a mismatch at `hooks/list` simply
-- refreshes the row.
CREATE TABLE codex_hook_trust (
    home          TEXT NOT NULL,
    key           TEXT NOT NULL,
    trusted_hash  TEXT NOT NULL,
    codex_version TEXT NOT NULL DEFAULT '',
    updated_at    TEXT NOT NULL,
    PRIMARY KEY (home, key)
);
