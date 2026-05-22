//! Shared HTTP response builders for the signaling HTTP servers, plus the
//! `result_json` helper that shapes tool-call payloads for both JSON-RPC
//! dispatchers (internal and external).

use crate::signaling::protocol::{
    JsonRpcError, JsonRpcRequest, JsonRpcResponse, ToolCallResult,
};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::{Response, StatusCode};
use serde_json::json;
use tracing::warn;

/// Wrap a serializable value as `ToolCallResult::text` with a JSON-string body.
/// `fallback` is the literal returned if serialization fails ("[]" for arrays,
/// "{}" for objects). Single source of truth shared between
/// `signaling::jsonrpc` and `signaling::external_jsonrpc`.
pub(super) fn result_json<T: serde::Serialize>(value: &T, fallback: &str) -> ToolCallResult {
    ToolCallResult::text(serde_json::to_string(value).unwrap_or_else(|_| fallback.into()))
}

/// Stub `{"ok": true}` response shape — the standard "operation succeeded,
/// nothing to return" payload used across the external MCP tool dispatch.
pub(super) fn ok_response() -> ToolCallResult {
    result_json(&serde_json::json!({ "ok": true }), "{}")
}

pub(super) fn text_response(status: StatusCode, body: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain")
        .body(Full::new(Bytes::from(body.to_owned())))
        .expect("static response")
}

pub(super) fn json_response<T: serde::Serialize>(
    status: StatusCode,
    body: &T,
) -> Response<Full<Bytes>> {
    let bytes = serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec());
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(bytes)))
        .expect("static response")
}

/// Collect the HTTP body and parse it as a JSON-RPC request. On success
/// returns the parsed `JsonRpcRequest`; on failure returns a fully-formed
/// `Response` ready to bubble out of the handler — either a 400 for body-read
/// errors or a 200 wrapping a JSON-RPC PARSE_ERROR (-32700) envelope.
///
/// Shared by `signaling::server` and `signaling::external_server`; their
/// `handle_request` functions had identical copies of this logic before F4.
pub(super) async fn decode_jsonrpc_body(
    body: Incoming,
) -> Result<JsonRpcRequest, Response<Full<Bytes>>> {
    let body_bytes = match body.collect().await {
        Ok(c) => c.to_bytes(),
        Err(err) => {
            warn!(?err, "read body");
            return Err(text_response(StatusCode::BAD_REQUEST, "body read error"));
        }
    };
    match serde_json::from_slice(&body_bytes) {
        Ok(rpc) => Ok(rpc),
        Err(err) => {
            warn!(?err, "json-rpc parse");
            Err(json_response(
                StatusCode::OK,
                &JsonRpcResponse::err(
                    json!(null),
                    JsonRpcError::new(JsonRpcError::PARSE_ERROR, "invalid JSON"),
                ),
            ))
        }
    }
}

/// Shape a dispatcher's outcome into the HTTP response both servers return:
/// notifications (`Ok(None)`) → 202 ACCEPTED with empty body; results
/// (`Ok(Some)`) → 200 with the JSON-RPC envelope; errors → 200 wrapping a
/// JSON-RPC error envelope using `id_for_err` (which should be the request's
/// original id or `null` if absent).
pub(super) fn dispatch_outcome_to_response(
    outcome: Result<Option<JsonRpcResponse>, JsonRpcError>,
    id_for_err: serde_json::Value,
) -> Response<Full<Bytes>> {
    match outcome {
        Ok(None) => text_response(StatusCode::ACCEPTED, ""),
        Ok(Some(resp)) => json_response(StatusCode::OK, &resp),
        Err(err) => json_response(StatusCode::OK, &JsonRpcResponse::err(id_for_err, err)),
    }
}
