//! End-to-end test of the native loop's MCP client against a real signaling
//! server.
//!
//! The unit tests in `agents::native::mcp_client` cover the pure helpers. This
//! file covers the thing they cannot: that the client and the server actually
//! agree on the wire dialect, and — the reason the native loop goes over HTTP
//! at all — that **role enforcement still applies through this path**.

use bot_hq::agents::native::mcp_client::{mcp_tools_to_anthropic, McpClient};
use bot_hq::signaling::{start_signaling_server, SignalingBridge};
use bot_hq::storage::Storage;
use std::sync::Arc;

fn client_for(addr: std::net::SocketAddr, agent: &str) -> McpClient {
    McpClient::new(format!("http://{addr}/sessions/s1/{agent}/mcp")).unwrap()
}

/// A bridge whose storage holds session `s1` with the roster every real session
/// is seeded with.
///
/// The gate resolves the caller's grants from `session_participants`, so a
/// bridge with no storage refuses every gated tool to every caller for the same
/// reason — which would let the two role tests below pass while distinguishing
/// nothing. Seeding is what keeps them about the roles.
async fn seeded_bridge() -> Arc<SignalingBridge> {
    let bridge = SignalingBridge::new();
    let storage = Storage::memory().await.unwrap();
    bridge.set_storage(storage.clone()).await;
    storage
        .create_session("s1", "native-mcp", None)
        .await
        .unwrap();
    storage.ensure_session_roster("s1").await.unwrap();
    bridge
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn initialize_round_trips() {
    let bridge = SignalingBridge::new();
    let server = start_signaling_server(Arc::clone(&bridge)).await.unwrap();
    let client = client_for(server.local_addr, "rain");

    let result = client.initialize().await.unwrap();
    assert_eq!(result["serverInfo"]["name"], "bot-hq-signaling");
    assert!(result["protocolVersion"].is_string());

    server.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn listed_tools_convert_into_valid_messages_api_entries() {
    let bridge = SignalingBridge::new();
    let server = start_signaling_server(Arc::clone(&bridge)).await.unwrap();
    let client = client_for(server.local_addr, "rain");

    let tools = client.list_tools().await.unwrap();
    assert!(!tools.is_empty(), "server advertises tools");

    let converted = mcp_tools_to_anthropic(&tools);
    assert_eq!(
        converted.len(),
        tools.len(),
        "every advertised tool survives conversion"
    );

    for t in &converted {
        assert!(t["name"].is_string());
        // The Messages API rejects a tool without `input_schema`; MCP spells it
        // `inputSchema`, so a missed rename here is a silent no-tool-calls bug.
        assert!(
            t["input_schema"].is_object(),
            "tool {} lost its schema in conversion",
            t["name"]
        );
        assert!(t.get("inputSchema").is_none());
    }

    let names: Vec<&str> = converted.iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(names.contains(&"cl_index_search"));
    assert!(names.contains(&"session_doc_write"));

    server.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hands_only_tools_are_still_denied_to_eyes_over_this_path() {
    let bridge = seeded_bridge().await;
    let server = start_signaling_server(Arc::clone(&bridge)).await.unwrap();
    let client = client_for(server.local_addr, "rain");

    let outcome = client
        .call_tool(
            "tu_1",
            "ask_user_choice",
            serde_json::json!({ "question": "q", "options": ["a"] }),
        )
        .await;

    assert!(
        outcome.is_error,
        "routing through HTTP must not bypass the tool gate"
    );
    // Named against EYES's actual grants rather than the role name: the gate
    // reads `session_participants.capabilities` now, and this path has to reach
    // the same verdict the in-process dispatch does.
    assert!(
        outcome.content.contains("needs the `ask_user` capability"),
        "got: {}",
        outcome.content
    );
    assert_eq!(outcome.tool_use_id, "tu_1");

    server.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn eyes_only_tools_are_still_denied_to_hands_over_this_path() {
    let bridge = seeded_bridge().await;
    let server = start_signaling_server(Arc::clone(&bridge)).await.unwrap();
    let client = client_for(server.local_addr, "brian");

    let outcome = client
        .call_tool("tu_2", "eyes_flag", serde_json::json!({}))
        .await;

    assert!(
        outcome.is_error,
        "routing through HTTP must not bypass the tool gate"
    );
    assert!(
        outcome.content.contains("needs the `file_finding` capability"),
        "got: {}",
        outcome.content
    );

    server.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dead_server_becomes_an_error_outcome_not_a_failed_turn() {
    // Nothing is listening here. The loop must be able to hand the model a
    // readable failure and carry on — errors are inputs to the loop.
    let client = McpClient::new("http://127.0.0.1:1/sessions/s1/rain/mcp").unwrap();

    let outcome = client
        .call_tool("tu_3", "cl_index_search", serde_json::json!({}))
        .await;

    assert!(outcome.is_error);
    assert_eq!(outcome.tool_use_id, "tu_3");
    assert!(!outcome.content.is_empty());
}
