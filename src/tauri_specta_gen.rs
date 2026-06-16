//! Composes the tauri-specta `Builder` for the full command set.
//!
//! Re-export TypeScript bindings via `Builder::export(...)` at startup so
//! the frontend's `frontend/src/lib/bindings.ts` stays in lockstep with
//! the Rust command signatures + `AppError` shape + view types. Runtime
//! emit lives in Batch 4's `main.rs`.
//!
//! Note: i64 fields (e.g. message ids) map to TS `number` via
//! `BigIntExportBehavior::Number`. JS numbers are float64 — values stay
//! exact up to 2^53. Sqlite ROWIDs we use are bounded well below that.

pub fn typescript_config() -> specta_typescript::Typescript {
    // `@ts-nocheck` suppresses tsc on the generated file itself. The frontend
    // tsconfig enables `noUnusedLocals`, and tauri-specta unconditionally
    // imports `TAURI_CHANNEL` + emits `__makeEvents__` whether commands use
    // them or not — both trip the rule. Type-checking still happens at the
    // call sites that import from this file; the file is a thin pass-through.
    specta_typescript::Typescript::default()
        .header("// @ts-nocheck")
        .bigint(specta_typescript::BigIntExportBehavior::Number)
}

use crate::tauri_cmd::{
    agent_configs, claude_config, cl, docs, messages, models, plugins, policy, sessions, tool_gate,
    tray, updates,
};
use tauri_specta::{collect_commands, Builder};

pub fn builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new().commands(collect_commands![
        // Sessions
        sessions::create_session,
        sessions::dispatch_session,
        sessions::get_session,
        sessions::get_session_project_info,
        sessions::check_session_dirty,
        sessions::session_worktree_kept,
        sessions::list_sessions,
        sessions::list_closed_sessions,
        sessions::respawn_session,
        sessions::restart_session,
        sessions::rename_session,
        sessions::get_session_phase,
        sessions::close_session,
        // Messages
        messages::get_session_messages,
        messages::broadcast_message,
        // Agent configs
        agent_configs::get_agent_config,
        agent_configs::list_agent_configs,
        agent_configs::upsert_agent_config,
        // Models registry + default-model setting
        models::list_models,
        models::upsert_model,
        models::delete_model,
        models::get_app_setting,
        models::set_app_setting,
        // CL
        cl::cl_index_search,
        cl::cl_folder_search,
        cl::cl_register_read,
        cl::cl_rescan,
        cl::list_projects,
        cl::cl_read_file,
        cl::cl_write_file,
        cl::cl_set_description,
        cl::cl_set_folder_description,
        cl::cl_delete_folder_description,
        cl::cl_register_project,
        cl::cl_unregister_project,
        cl::cl_create_project,
        cl::cl_delete_project,
        cl::cl_rename_project,
        cl::cl_create_file,
        cl::cl_mkdir,
        cl::cl_rename,
        cl::cl_delete_path,
        // Tool Gate (global gated-Bash keywords)
        tool_gate::get_tool_gate_keywords,
        tool_gate::set_tool_gate_keywords,
        // Policy (3-tier toggles: global / project / session — user-only)
        policy::get_general_policy,
        policy::set_general_policy,
        policy::get_project_policy,
        policy::set_project_policy,
        policy::get_session_policy,
        policy::set_session_policy,
        policy::get_session_tool_gate,
        policy::set_session_tool_gate,
        policy::read_violations,
        // Claude Config (surface + override the config agents inherit)
        claude_config::claude_config_read,
        claude_config::get_claude_overrides,
        claude_config::set_claude_overrides,
        claude_config::claude_config_set_string,
        claude_config::claude_config_set_bool,
        claude_config::claude_config_set_plugin_enabled,
        // Tray (choices / approvals / halts)
        tray::resolve_choice,
        tray::list_session_tray,
        tray::list_pending_tray,
        // Session documents
        docs::session_doc_search,
        docs::session_doc_read,
        docs::compute_apply_diff,
        docs::summarize_session_doc,
        docs::validate_model,
        // Plugins
        plugins::install_plugin,
        plugins::list_installed_plugins,
        plugins::enable_plugin,
        plugins::disable_plugin,
        plugins::uninstall_plugin,
        // Updates (check GitHub Releases for a newer bot-hq)
        updates::check_for_update,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_constructs_with_full_command_set() {
        let _b = builder();
    }

    #[test]
    fn builder_exports_to_typescript() {
        let b = builder();
        let out = std::env::temp_dir().join("bot-hq-types-batch2.ts");
        b.export(typescript_config(), &out)
            .expect("tauri-specta export must succeed");
        assert!(out.exists());
        let body = std::fs::read_to_string(&out).expect("read generated TS");
        // Sanity: a few of the command names should appear in the bindings.
        assert!(body.contains("createSession") || body.contains("create_session"));
    }
}
