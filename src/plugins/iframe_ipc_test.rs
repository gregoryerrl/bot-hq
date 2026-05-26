//! Coverage gap closed: dummy iframe origin + capability set verifying the
//! full chain (capability JSON gen → command-allow check). Per design doc
//! § Testing.

use crate::plugins::capabilities::CapabilityGen;
use crate::plugins::loader::LoadedPlugin;
use crate::plugins::manifest::PluginManifest;
use std::path::PathBuf;

fn plugin_with(id: &str, caps: &[&str]) -> LoadedPlugin {
    LoadedPlugin {
        manifest: PluginManifest {
            id: id.to_string(),
            name: id.to_string(),
            version: "0.1.0".to_string(),
            entry: "i.html".to_string(),
            requested_capabilities: caps.iter().map(|s| s.to_string()).collect(),
            slots: vec![],
        },
        dir: PathBuf::from("/dev/null"),
    }
}

#[test]
fn iframe_origin_matches_capability_remote_url() {
    let p = plugin_with("discord", &[]);
    let origin = p.manifest.iframe_origin();
    let cap = CapabilityGen::for_plugin(&p);
    let expected_origin_pattern = format!("{origin}/*");
    assert!(cap.remote.urls.contains(&expected_origin_pattern));
}

#[test]
fn requested_capability_maps_to_allow_permission() {
    let p = plugin_with("discord", &["cl_index_search", "session_doc_search"]);
    let cap = CapabilityGen::for_plugin(&p);
    assert!(cap.permissions.contains(&"allow-cl-index-search".to_string()));
    assert!(cap.permissions.contains(&"allow-session-doc-search".to_string()));
}

#[test]
fn iframe_origin_denies_uncapped_command() {
    let p = plugin_with("discord", &["cl_index_search"]);
    assert!(CapabilityGen::is_command_allowed(&p, "cl_index_search"));
    assert!(!CapabilityGen::is_command_allowed(&p, "create_session"));
    assert!(!CapabilityGen::is_command_allowed(&p, "grant_session_permission"));
}

#[test]
fn empty_capability_set_denies_everything() {
    let p = plugin_with("limited", &[]);
    assert!(!CapabilityGen::is_command_allowed(&p, "cl_index_search"));
    assert!(!CapabilityGen::is_command_allowed(&p, "create_session"));
}

#[test]
fn capability_identifier_is_plugin_scoped() {
    let cap_a = CapabilityGen::for_plugin(&plugin_with("a", &[]));
    let cap_b = CapabilityGen::for_plugin(&plugin_with("b", &[]));
    assert_ne!(cap_a.identifier, cap_b.identifier);
    assert!(cap_a.identifier.contains("a"));
    assert!(cap_b.identifier.contains("b"));
}
