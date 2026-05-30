"""Thin JSON-RPC client for bot-hq's external driver MCP.

Endpoint : POST http://127.0.0.1:7892/mcp
Auth     : Authorization: Bearer <token>   (token at <data_dir>/mcp-token, 0600)
Wire     : JSON-RPC 2.0. A `tools/call` result is DOUBLE-ENCODED — the real
           payload is a JSON string inside result.content[0].text. See unwrap().

Stdlib only (urllib) so the harness runs on a bare interpreter with no pip
installs — important on bleeding-edge Python where heavy wheels may not build.

Sources: src/signaling/external_server.rs (route /mcp + bearer),
         src/signaling/external_jsonrpc.rs (tool dispatch + return shapes),
         src/signaling/protocol.rs:549 (ToolCallResult), response.rs:19.
"""
from __future__ import annotations

import itertools
import json
import os
import pathlib
import urllib.error
import urllib.request
from typing import Any, Optional


class BotHqError(RuntimeError):
    """Any transport / protocol / tool error from the external MCP."""


def default_token_path() -> pathlib.Path:
    data_dir = os.environ.get("BOT_HQ_DATA_DIR")
    base = pathlib.Path(data_dir).expanduser() if data_dir else pathlib.Path.home() / ".bot-hq"
    return base / "mcp-token"


def unwrap(envelope: dict) -> Any:
    """Decode a tools/call envelope to the real payload.

    Payloads are double-encoded: once as a JSON string at
    result.content[0].text, once in the JSON-RPC envelope. Skipping this and
    treating result as the data yields garbage (an MCP content list).
    """
    try:
        text = envelope["result"]["content"][0]["text"]
    except (KeyError, IndexError, TypeError) as e:
        raise BotHqError(f"malformed tool-call envelope ({e}): {envelope!r}")
    if text == "":
        return {}
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        return text  # tolerate a plain-text payload


class BotHqClient:
    def __init__(
        self,
        url: str = "http://127.0.0.1:7892/mcp",
        token: Optional[str] = None,
        token_path: Optional[str] = None,
        timeout: float = 70.0,
    ):
        if token is None:
            p = pathlib.Path(token_path).expanduser() if token_path else default_token_path()
            try:
                token = p.read_text().strip()
            except OSError as e:
                raise BotHqError(f"cannot read MCP token at {p}: {e}")
        self.url = url
        self.timeout = timeout
        self._ids = itertools.count(1)
        self._auth = f"Bearer {token}"

    # ---- transport ----
    def _rpc(self, method: str, params: Optional[dict] = None, *, timeout: Optional[float] = None) -> dict:
        body = {"jsonrpc": "2.0", "id": next(self._ids), "method": method}
        if params is not None:
            body["params"] = params
        data = json.dumps(body).encode()
        req = urllib.request.Request(
            self.url,
            data=data,
            method="POST",
            headers={"Authorization": self._auth, "Content-Type": "application/json"},
        )
        try:
            with urllib.request.urlopen(req, timeout=timeout or self.timeout) as resp:
                raw = resp.read()
        except urllib.error.HTTPError as e:
            detail = e.read().decode(errors="replace")[:200]
            if e.code == 401:
                raise BotHqError("401 Unauthorized — bad/stale MCP bearer token")
            raise BotHqError(f"HTTP {e.code} from {method}: {detail}")
        except urllib.error.URLError as e:
            raise BotHqError(f"cannot reach bot-hq external MCP at {self.url}: {e.reason} "
                             "(is the app running with its window open?)")
        env = json.loads(raw)
        if env.get("error"):
            raise BotHqError(f"JSON-RPC error from {method}: {env['error']}")
        return env

    def call(self, tool: str, arguments: Optional[dict] = None, *, timeout: Optional[float] = None) -> Any:
        return unwrap(self._rpc("tools/call", {"name": tool, "arguments": arguments or {}}, timeout=timeout))

    def initialize(self) -> dict:
        return self._rpc("initialize", {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "swebench-harness", "version": "0.1"},
        })["result"]

    # ---- typed tool wrappers (only the ones the harness uses) ----
    def get_status(self) -> dict:
        return self.call("get_status")

    def get_agent_configs(self) -> list:
        return self.call("get_agent_configs")["agent_configs"]

    def create_session(self, title: str, working_repo_path: Optional[str] = None) -> str:
        args: dict = {"title": title}
        if working_repo_path:
            args["working_repo_path"] = str(working_repo_path)
        return self.call("create_session", args, timeout=120.0)["session_id"]

    def send_message(self, session_id: str, text: str) -> None:
        self.call("send_message", {"session_id": session_id, "text": text})

    def wait_for_change(self, session_id: str, since_id: Optional[int] = None, timeout_ms: int = 30000) -> list:
        args: dict = {"session_id": session_id, "timeout_ms": timeout_ms}
        if since_id is not None:
            args["since_id"] = since_id
        # HTTP read timeout must outlast the server-side long-poll.
        return self.call("wait_for_change", args, timeout=timeout_ms / 1000 + 15)["messages"]

    def get_session_snapshot(self, session_id: str, msg_limit: int = 50) -> dict:
        return self.call("get_session_snapshot", {"session_id": session_id, "msg_limit": msg_limit})

    def get_pending_choices(self) -> list:
        return self.call("get_pending_choices")["pending_choices"]

    def resolve_choice(self, choice_id: str, picked: str) -> None:
        self.call("resolve_choice", {"choice_id": choice_id, "picked": picked})

    def close_session(self, session_id: str, archive: bool = True) -> None:
        self.call("close_session", {"session_id": session_id, "archive": archive})
