//! Shared HTTP response builders for the signaling HTTP servers, plus the
//! `result_json` helper that shapes tool-call payloads for both JSON-RPC
//! dispatchers (internal and external).

use crate::signaling::protocol::ToolCallResult;
use bytes::Bytes;
use http_body_util::Full;
use hyper::{Response, StatusCode};

/// Wrap a serializable value as `ToolCallResult::text` with a JSON-string body.
/// `fallback` is the literal returned if serialization fails ("[]" for arrays,
/// "{}" for objects). Single source of truth shared between
/// `signaling::jsonrpc` and `signaling::external_jsonrpc`.
pub(super) fn result_json<T: serde::Serialize>(value: &T, fallback: &str) -> ToolCallResult {
    ToolCallResult::text(serde_json::to_string(value).unwrap_or_else(|_| fallback.into()))
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
