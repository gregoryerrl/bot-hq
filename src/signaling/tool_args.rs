//! Shared JSON-RPC argument-extraction helpers for the `jsonrpc::call_tool`
//! dispatch table. "Shared" is now historical: the second caller was
//! `external_jsonrpc::call_external_tool`, deleted with the external driver
//! (2026-08-17).

use crate::signaling::protocol::JsonRpcError;
use serde_json::Value;

/// Extract a required string argument. Returns INVALID_PARAMS if absent or non-string.
pub(super) fn arg_required_str(args: &Value, key: &str) -> Result<String, JsonRpcError> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| JsonRpcError::new(JsonRpcError::INVALID_PARAMS, format!("missing {key}")))
}

/// Extract an optional string argument. None if absent or non-string.
pub(super) fn arg_opt_str(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Extract a required array-of-strings argument. Returns INVALID_PARAMS if absent or
/// non-array; non-string entries are silently dropped.
pub(super) fn arg_required_str_array(
    args: &Value,
    key: &str,
) -> Result<Vec<String>, JsonRpcError> {
    let arr = args
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| JsonRpcError::new(JsonRpcError::INVALID_PARAMS, format!("missing {key}")))?;
    Ok(arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
}
