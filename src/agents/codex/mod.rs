//! The Codex connector: OpenAI's Codex CLI driven as one `codex app-server`
//! process per participant over stdio JSON-RPC, behind the same
//! `AgentHandle` / `AgentEvent` seam claude-code's `spawn.rs` fills (rc3 D9
//! amended 2026-10-21, the user's ratification on tray 87c5309d).
//!
//! `home` — the bot-hq-managed Codex home: the hooks file bot-hq writes into
//! it and the trust it grants that file per spawn.

pub mod home;
