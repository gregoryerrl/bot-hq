# bot-hq telemetry ingest (Cloudflare Worker + D1)

The opt-in diagnostics sink. bot-hq clients POST small batches (errors/panics
as hashes, OS + version, feature counters — never repo or prompt content) to
`/v1/events`; the Worker validates, throttles and inserts into a D1 table you
own. There is no public read path — querying is yours alone, via `wrangler d1`
or the Cloudflare dashboard.

**Released binaries default to the maintainer's deployment of this worker**
(`core/telemetry.rs::DEFAULT_ENDPOINT`, deployed 2026-08-25). To self-host
instead, deploy your own below and paste its URL into **Settings →
Diagnostics** — the setting overrides the default. (The `database_id` in
`wrangler.toml` is the maintainer's; `d1 create` prints yours — replace it.)

## One-time deploy (free tier)

```sh
cd packaging/telemetry-worker
npm install
npx wrangler login                                   # opens the browser once
npx wrangler d1 create bot-hq-telemetry              # copy the database_id it prints
# paste that id into wrangler.toml (database_id = "…")
npx wrangler d1 execute bot-hq-telemetry --remote --file=schema.sql
npx wrangler deploy                                  # prints https://bot-hq-telemetry.<you>.workers.dev
```

Then in bot-hq: **Settings → Diagnostics → endpoint** — paste the printed URL.
`GET <url>/health` answering `ok` proves the deploy from any browser.

## Querying what came in

```sh
npx wrangler d1 execute bot-hq-telemetry --remote \
  --command "SELECT kind, COUNT(*) FROM events GROUP BY kind"
```

## Notes

- The endpoint URL ships inside released binaries, so treat the route as
  public: caps + a per-IP throttle are in `src/index.ts`, and the free-tier D1
  write quota is the final backstop. Junk rows are possible; reading your data
  is not.
- `npm test` runs the pure validation/throttle suite (vitest); no Cloudflare
  account needed for tests.
