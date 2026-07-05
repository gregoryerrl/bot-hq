//! The grantable-command catalog — the versioned contract between plugins
//! and the host (plugin runtime v1, `api_version: 1`).
//!
//! A plugin's manifest requests entries by name in `requested_capabilities`;
//! install/preview rejects names that aren't in the catalog, and
//! `plugin_invoke_proxy` (tauri_cmd/plugin_api.rs) dispatches ONLY through
//! this list — there is no arbitrary-command path from a plugin iframe.
//! Descriptions feed the install-consent UI, so write them for the user
//! deciding whether to grant, not for developers.
//!
//! v1 is read-first with two deliberate exceptions: the plugin's own
//! namespaced key/value store, and `spawn_session` — session CREATION,
//! double-consented (install-time grant + a per-spawn host confirm dialog),
//! which reaches only a NEW session's stdin with a prompt the user saw and
//! approved. CL writes stay user/agent-only (propose-don't-mutate), and
//! nothing here reaches EXISTING sessions, their stdin, or policy.

/// One grantable command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogEntry {
    pub name: &'static str,
    /// Consent-screen copy: what the user is granting, in their terms.
    pub description: &'static str,
}

pub const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        name: "list_sessions",
        description: "See the list of active sessions (titles, repos, status)",
    },
    CatalogEntry {
        name: "get_session",
        description: "Read one session's details (title, repo, models, timestamps)",
    },
    CatalogEntry {
        name: "list_messages",
        description: "Read a session's chat history (user + agent messages)",
    },
    CatalogEntry {
        name: "session_doc_search",
        description: "Read a session's I/P/A/V phase documents",
    },
    CatalogEntry {
        name: "cl_index_search",
        description: "See which Context Library files exist (names + descriptions)",
    },
    CatalogEntry {
        name: "cl_folder_search",
        description: "See Context Library folder descriptions",
    },
    CatalogEntry {
        name: "cl_retrieve",
        description: "Search and read the best-matching Context Library sections",
    },
    CatalogEntry {
        name: "cl_read_file",
        description: "Read whole Context Library files",
    },
    CatalogEntry {
        name: "list_projects",
        description: "See the list of registered projects",
    },
    CatalogEntry {
        name: "compute_apply_diff",
        description: "Read a session's code changes (git diff)",
    },
    CatalogEntry {
        name: "spawn_session",
        description: "Open new agent sessions with a prompt you will see and approve each time",
    },
    CatalogEntry {
        name: "plugin_kv_get",
        description: "Read this plugin's own saved settings/state",
    },
    CatalogEntry {
        name: "plugin_kv_set",
        description: "Save this plugin's own settings/state",
    },
];

pub fn is_valid(command: &str) -> bool {
    CATALOG.iter().any(|e| e.name == command)
}

pub fn describe(command: &str) -> Option<&'static str> {
    CATALOG.iter().find(|e| e.name == command).map(|e| e.description)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_names_resolve_and_describe() {
        for entry in CATALOG {
            assert!(is_valid(entry.name));
            assert_eq!(describe(entry.name), Some(entry.description));
        }
    }

    #[test]
    fn unknown_and_hostlike_names_are_invalid() {
        for bad in [
            "create_session",     // raw creation — only spawn_session's double-consent door exists
            "broadcast_message",  // reaches EXISTING agents' stdin — never grantable
            "cl_write_file",      // CL canon mutates only by user action
            "install_plugin",     // plugins must not install plugins
            "close_session",      // existing-session control — never grantable
            "",
        ] {
            assert!(!is_valid(bad), "{bad:?} must not be grantable");
            assert_eq!(describe(bad), None);
        }
    }

    #[test]
    fn catalog_names_are_unique() {
        let mut names: Vec<_> = CATALOG.iter().map(|e| e.name).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len());
    }
}
