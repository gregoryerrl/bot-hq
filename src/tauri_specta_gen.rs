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
    agent_configs, cl, docs, messages, plugins, policy, questions, screenshot, sessions, tool_gate,
};
use tauri_specta::{collect_commands, Builder};

pub fn builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new().commands(collect_commands![
        // Sessions
        sessions::create_session,
        sessions::get_session,
        sessions::list_sessions,
        sessions::respawn_session,
        sessions::get_session_phase,
        sessions::advance_session_phase,
        sessions::close_session,
        // Messages
        messages::get_session_messages,
        messages::broadcast_message,
        // Agent configs
        agent_configs::get_agent_config,
        agent_configs::list_agent_configs,
        agent_configs::upsert_agent_config,
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
        cl::cl_create_file,
        cl::cl_mkdir,
        cl::cl_rename,
        cl::cl_delete_path,
        // Policy / session permissions
        policy::grant_session_permission,
        policy::revoke_session_permission,
        policy::list_session_permissions,
        // Tool Gate (global gated-Bash keywords)
        tool_gate::get_tool_gate_keywords,
        tool_gate::set_tool_gate_keywords,
        // Questions / choices
        questions::list_pending_choices,
        questions::resolve_choice,
        // Session documents
        docs::session_doc_search,
        docs::session_doc_read,
        docs::compute_apply_diff,
        // Screenshot
        screenshot::capture_window_screenshot,
        // Plugins
        plugins::install_plugin,
        plugins::list_installed_plugins,
        plugins::enable_plugin,
        plugins::disable_plugin,
        plugins::uninstall_plugin,
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
