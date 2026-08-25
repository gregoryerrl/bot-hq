# bot-hq — diagnostics & privacy

**By default, bot-hq sends nothing anywhere.** Diagnostics are strictly
opt-in: nothing is collected or transmitted until you enable them, either on
the one-time first-run card or in **Settings → Diagnostics**. Disable them
there at any time.

## What is sent when you opt in

- **`app_launch`** — the app version, operating system and CPU architecture
  (e.g. `1.0.0`, `macos`, `aarch64`).
- **`panic`** — when the app crashes: SHA-256 **hashes** of the panic message
  and backtrace. The text itself never leaves your machine, and your home
  directory path is redacted to `~` before hashing. Hashes let identical
  crashes be counted without revealing their content.
- **`error`** — a short error class and context tag (e.g. `spawn`,
  `agent_start`) from explicit call sites.

**Never sent:** repository content, code, prompts, chat, session transcripts,
Context Library content, file paths, tokens or credentials. There is no
passive "session log upload" — sessions stay on your machine.

## Where it goes

Diagnostics POST to an ingest endpoint **operated by the bot-hq author
(Gregory Errl Babela) on Cloudflare** (a Worker writing to a D1 database) —
by default `https://bot-hq-telemetry.gregoryerrl.workers.dev`. The endpoint
has no public read path.

**Self-hosting:** the sink's full source ships in this repository at
[`packaging/telemetry-worker/`](packaging/telemetry-worker/). Deploy it to
your own Cloudflare account and paste your URL into **Settings →
Diagnostics** — the setting overrides the default, and your data then goes
only to you.

## The install id

Enabling diagnostics mints a random UUID for this install. It is **stable
while enabled** — that is what distinguishes one install crashing five
hundred times from five hundred installs crashing once — and it is
**deleted when you disable**, along with anything queued but unsent.
Re-enabling mints a fresh id; data sent under an old id cannot be linked to
the new one. The id identifies an installation, never a person: no email,
username or machine name accompanies it.

## Inspecting before it ships

Events queue locally in plain JSONL at `~/.bot-hq/.local/telemetry.jsonl`
(capped at 1 MB, oldest dropped first) and ship in batches. Open the file to
see exactly what would be sent; delete it or disable diagnostics to keep it
from shipping.
