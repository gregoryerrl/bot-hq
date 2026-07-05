/**
 * bhq-sdk.js — the plugin side of the bot-hq postMessage channel
 * (plugin runtime v1, api_version 1).
 *
 * Copy this file into your plugin bundle and load it from your entry HTML.
 * It exposes `window.BHQ` (and ES-module exports) with:
 *
 *   BHQ.invoke(cmd, args?) -> Promise<result>
 *     Call a granted catalog command. `cmd` must be listed in your
 *     manifest's requested_capabilities AND in the host catalog — the host
 *     enforces both in Rust; an ungranted call rejects.
 *
 *   BHQ.onEvent(topic, cb) -> unsubscribe()
 *     Subscribe to host push events. v1 topics: "plugin_assets_changed"
 *     (a file in YOUR served directory changed — refresh your content) and
 *     "sessions_changed" (fires only if you hold the list_sessions grant).
 *
 *   BHQ.nonce
 *     The mount nonce the host embedded in your URL (?bhq=…). The SDK
 *     attaches it to every message automatically; exposed for debugging.
 *
 * The SDK also auto-answers the host's 5s `bhq:ping` heartbeat. If your
 * plugin stops ponging (crashed tab, infinite loop), the host tears the
 * iframe down and offers the user a Reload.
 *
 * Message contract (plugin->host messages carry the mount nonce):
 *   plugin -> host: { type: "bhq:invoke", id, cmd, args?, nonce }
 *   host -> plugin: { type: "bhq:result", id, ok, data | error }
 *   host -> plugin: { type: "bhq:event", topic }
 *   host -> plugin: { type: "bhq:ping" }
 *   plugin -> host: { type: "bhq:pong", nonce }
 *
 * Replies from the host arrive with a platform-dependent origin (the shell
 * runs on tauri://localhost, http://localhost:1420 in dev, etc.), so the
 * SDK correlates strictly by request id — ids are issued locally and never
 * guessable by other frames, and the iframe's own document is the only
 * code that can receive messages posted to this window.
 */

const NONCE = new URLSearchParams(window.location.search).get("bhq") ?? "";

let seq = 0;
const pending = new Map();
const eventSubs = new Map(); // topic -> Set<cb>

window.addEventListener("message", (event) => {
  const msg = event.data;
  if (!msg || typeof msg !== "object") return;

  if (msg.type === "bhq:ping") {
    // Heartbeat: answer immediately so the host knows we're alive.
    window.parent.postMessage({ type: "bhq:pong", nonce: NONCE }, "*");
    return;
  }

  if (msg.type === "bhq:event" && typeof msg.topic === "string") {
    for (const cb of eventSubs.get(msg.topic) ?? []) {
      try {
        cb();
      } catch (e) {
        console.error(`bhq onEvent(${msg.topic}) handler threw`, e);
      }
    }
    return;
  }

  if (msg.type === "bhq:result" && pending.has(msg.id)) {
    const { resolve, reject } = pending.get(msg.id);
    pending.delete(msg.id);
    if (msg.ok) {
      resolve(msg.data);
    } else {
      reject(new Error(msg.error ?? "plugin invoke failed"));
    }
  }
});

/**
 * Invoke a granted catalog command on the host.
 * @param {string} cmd  Catalog command name (e.g. "list_sessions").
 * @param {object} [args]  Command args; see docs/PLUGINS.md per command.
 * @returns {Promise<unknown>} the command's JSON result
 */
export function invoke(cmd, args) {
  return new Promise((resolve, reject) => {
    const id = `r${++seq}`;
    pending.set(id, { resolve, reject });
    window.parent.postMessage(
      { type: "bhq:invoke", id, cmd, args, nonce: NONCE },
      "*",
    );
  });
}

/**
 * Subscribe to a host push event ("plugin_assets_changed" |
 * "sessions_changed"). Returns an unsubscribe function.
 * @param {string} topic
 * @param {() => void} cb
 */
export function onEvent(topic, cb) {
  if (!eventSubs.has(topic)) eventSubs.set(topic, new Set());
  eventSubs.get(topic).add(cb);
  return () => eventSubs.get(topic)?.delete(cb);
}

export const nonce = NONCE;

// Global for plugins that skip modules and just <script src="bhq-sdk.js">.
window.BHQ = { invoke, onEvent, nonce: NONCE };
