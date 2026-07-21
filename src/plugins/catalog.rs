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
//! v1 is read-first with three deliberate write exceptions, each narrowly
//! fenced:
//!   - the plugin's own namespaced key/value store;
//!   - `spawn_session` — session CREATION only, double-consented (install
//!     grant + a per-spawn host confirm dialog), reaching only a NEW
//!     session's stdin with a prompt the user saw and approved;
//!   - `plugin_sessions` — a plugin creating AND driving its OWN helper
//!     sessions (send / wait / read / close). Unlike spawn_session this
//!     reaches EXISTING sessions' stdin, so it is fenced by OWNERSHIP: every
//!     `plugin_session_*` sub-command is gated on `created_by_plugin == this
//!     plugin` (`require_owned_session`), so a plugin can never see, message,
//!     or close the user's own sessions or another plugin's. Consent is the
//!     install-time grant (no per-call dialog — the ownership fence plus
//!     dashboard visibility are the guardrails).
//! CL writes stay user/agent-only — never a plugin surface. The raw
//! `create_session` / `broadcast_message` / `close_session` names stay
//! non-grantable — plugins reach sessions ONLY through the fenced arms above.

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
        name: "plugin_sessions",
        description: "Open its own helper agent sessions and drive them — send messages, read the replies, and close them. Strictly limited to sessions this plugin creates itself: it can never see, message, or close your other sessions or another plugin's. Each runs a single agent by default and appears in your dashboard.",
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

/// Sub-commands dispatched under a single bundled capability: the manifest
/// requests the CAPABILITY (a CATALOG entry, left); the iframe dispatches the
/// SUB-COMMANDS (right). Bundling fits a cohesive power whose parts are
/// useless individually — every `plugin_session_*` sub-command is
/// ownership-fenced to sessions the plugin itself created, so granting them
/// separately would be false granularity. Sub-commands are
/// dispatchable-but-not-grantable: they never appear in a manifest (only
/// `plugin_sessions` does), so they are absent from `CATALOG` / `is_valid`.
const BUNDLED_COMMANDS: &[(&str, &str)] = &[
    ("plugin_session_create", "plugin_sessions"),
    ("plugin_session_send", "plugin_sessions"),
    ("plugin_session_wait", "plugin_sessions"),
    ("plugin_session_messages", "plugin_sessions"),
    ("plugin_session_close", "plugin_sessions"),
];

/// The grantable capability a dispatch command requires. Identity for the 1:1
/// catalog commands (an existing command still matches its own grant); the
/// bundling capability for a sub-command. `check_plugin_grant` matches THIS
/// against the consent-frozen grant set.
pub fn required_capability(command: &str) -> &str {
    BUNDLED_COMMANDS
        .iter()
        .find(|(sub, _)| *sub == command)
        .map(|(_, cap)| *cap)
        .unwrap_or(command)
}

/// Is `command` legal to DISPATCH? True for grantable catalog capabilities AND
/// for bundled sub-commands. Grantability is stricter (`is_valid`, used by
/// install/consent): a sub-command is dispatchable but not independently
/// grantable.
pub fn is_dispatchable(command: &str) -> bool {
    is_valid(command) || BUNDLED_COMMANDS.iter().any(|(sub, _)| *sub == command)
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
            "cl_write_file",      // CL writes are user/agent surfaces, never plugin
            "install_plugin",     // plugins must not install plugins
            "close_session",      // existing-session control — never grantable
            "",
        ] {
            assert!(!is_valid(bad), "{bad:?} must not be grantable");
            assert!(!is_dispatchable(bad), "{bad:?} must not be dispatchable");
            assert_eq!(describe(bad), None);
        }
    }

    /// The `plugin_sessions` capability is grantable; its sub-commands are
    /// dispatchable but NOT independently grantable (they never appear in a
    /// manifest), and each maps back to the bundling capability the gate
    /// checks the grant against.
    #[test]
    fn plugin_sessions_bundle_maps_subcommands_to_the_capability() {
        assert!(is_valid("plugin_sessions"));
        assert!(is_dispatchable("plugin_sessions"));
        assert!(describe("plugin_sessions").is_some());
        assert_eq!(required_capability("plugin_sessions"), "plugin_sessions");

        for sub in [
            "plugin_session_create",
            "plugin_session_send",
            "plugin_session_wait",
            "plugin_session_messages",
            "plugin_session_close",
        ] {
            assert!(is_dispatchable(sub), "{sub:?} must be dispatchable");
            assert!(!is_valid(sub), "{sub:?} must not be independently grantable");
            assert_eq!(describe(sub), None, "{sub:?} is not a consent entry");
            assert_eq!(
                required_capability(sub),
                "plugin_sessions",
                "{sub:?} must require the plugin_sessions grant"
            );
        }
    }

    /// Load-bearing: `required_capability` MUST be identity for every 1:1
    /// catalog command. If it ever returned a different name, that command's
    /// existing grant would silently stop matching (a plugin granted
    /// `list_sessions` could no longer call it). Guards the mapping change to
    /// `check_plugin_grant`.
    #[test]
    fn required_capability_is_identity_for_catalog_commands() {
        for entry in CATALOG {
            assert_eq!(
                required_capability(entry.name),
                entry.name,
                "catalog command {:?} must map to itself",
                entry.name
            );
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
