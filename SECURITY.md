# Security

## Reporting a vulnerability

Please do not open a public issue for a security problem.

- Use GitHub's private vulnerability reporting on this repository
  (**Security → Report a vulnerability**), which reaches the maintainer
  without a public trace. It is the only reporting channel.
- Include the version (Settings → Updates shows the installed one, or the
  release tag), the platform,
  and a reproduction. A proof-of-concept against your own machine is welcome;
  please do not test against anyone else's.
- You will get an acknowledgement within a few days. Fixes ship in the next
  release and are called out under **Security** in `CHANGELOG.md`.

## Supported versions

Only the latest release receives fixes. bot-hq is a single-user desktop app
with no server component of ours behind it; upgrading is the fix.

## The trust boundary

bot-hq runs AI coding agents against your repositories, on your machine, as
you. Knowing what it trusts tells you what a report is about — and what is
working as designed.

**Trusted: the local user.** Every process running as your user account is
inside the boundary. Agents hold `Bash`; so does anything else you run. The
two loopback listeners (the MCP signaling server and the LLM proxy) bind
`127.0.0.1` on an ephemeral port and are meant for bot-hq's own subprocesses.
They refuse browser-originated requests (`Origin` / `Sec-Fetch-*`), the
signaling routes require a per-agent secret minted at spawn, the hook routes
require a per-launch secret, and the proxy forwards only to an upstream a
spawn resolved from your model settings this launch — that allow-list never
shrinks while the app runs, including an upstream you have since edited
away, so treat the model table as configuration with teeth. A request from
another local process holding those secrets is you.

**Trusted: a registered repository.** Registering a repository and opening a
session on it is running its code. bot-hq disables the git config hooks it
can on its own invocations (`core.fsmonitor`, hooks on `worktree add`,
external diff and textconv drivers), but `.gitattributes` filters
(`filter.<name>.process` / `.smudge`) still execute on any command that
checks out content — disabling them would break Git LFS — and the agents run
whatever the repository's tooling runs. Do not register a repository you
would not run `make` in.

**Untrusted: plugins.** A plugin is a static bundle served in a sandboxed
iframe under its own origin. It reaches bot-hq only through a per-call
capability check against the grants you approved at install, and its
manifest may name only a relative entry inside its own directory. A plugin
that can do something you did not grant is a vulnerability — report it.

**Untrusted: the network.** bot-hq makes outbound requests only to the model
provider (or the gateway you configured), GitHub's release API for the update
check, and — if you opted in — the telemetry endpoint named in `PRIVACY.md`,
which receives hashed diagnostics and never repository content. It accepts
no inbound connections from outside the machine.

**Known, accepted for now.** Model auth tokens are stored in plaintext SQLite
under `<data_dir>/.local/` with user-only permissions, and are placed in each
agent's environment — a backup of the data directory or an agent with `Bash`
can read them (README "Security caveats"; the OS keychain is planned).
Approved gated commands are recorded verbatim in `violations.jsonl`. Release
builds are unsigned on macOS and Windows; the Homebrew cask pins the DMG's
SHA-256.
