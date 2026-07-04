/**
 * Host side of the plugin postMessage RPC channel (plugin runtime v1).
 *
 * A plugin iframe never talks to Tauri. It postMessages the shell; this
 * module validates each message (source window + origin + mount nonce) and
 * forwards invokes to the single Rust enforcement point
 * (`plugin_invoke_proxy`). The heartbeat rides the same channel: a 5s
 * `bhq:ping` → the plugin SDK auto-answers `bhq:pong`; the backend sweep
 * turns unanswered pings into Slow/Crashed + the `plugin:crashed` event.
 *
 * The checks here are transport hygiene — real capability enforcement is
 * Rust-side. See docs/PLUGINS.md for the message contract plugin authors
 * target.
 */
import { convertFileSrc } from "@tauri-apps/api/core";
import { commands } from "./bindings";

export const PING_INTERVAL_MS = 5_000;

/** How this platform's webview surfaces the custom scheme. */
export type SchemeForm = "unix" | "windows";

/**
 * Decide the scheme form from a `convertFileSrc` sample. macOS/Linux
 * webviews keep custom schemes verbatim (`bhq-plugin://…`); Windows folds
 * them into an `https://<scheme>.localhost/…` origin.
 */
export function detectSchemeForm(sample: string): SchemeForm {
  return sample.startsWith("bhq-plugin://") ? "unix" : "windows";
}

export function schemeForm(): SchemeForm {
  return detectSchemeForm(convertFileSrc("probe", "bhq-plugin"));
}

/**
 * The iframe src for a plugin entry. The id rides the URL HOST on unix
 * webviews and the first PATH segment on Windows — `plugins::serve`'s
 * `parse_plugin_request` accepts both. The nonce authenticates the mount:
 * only a document loaded from this exact URL can echo it back.
 */
export function pluginEntryUrl(
  form: SchemeForm,
  pluginId: string,
  entry: string,
  nonce: string,
): string {
  const path = entry.replace(/^\/+/, "");
  return form === "unix"
    ? `bhq-plugin://${pluginId}/${path}?bhq=${nonce}`
    : `https://bhq-plugin.localhost/${pluginId}/${path}?bhq=${nonce}`;
}

/**
 * Acceptable `event.origin` values for a plugin's messages. "null" is
 * accepted because WKWebView can treat custom-scheme documents as an
 * opaque origin — the source-window + nonce checks carry authentication
 * there (and everywhere; origin is belt-and-braces).
 */
export function expectedOrigins(form: SchemeForm, pluginId: string): string[] {
  return form === "unix"
    ? [`bhq-plugin://${pluginId}`, "null"]
    : ["https://bhq-plugin.localhost", "null"];
}

export interface BhqInvokeMsg {
  type: "bhq:invoke";
  /** Correlation id, echoed on the reply. */
  id: string;
  cmd: string;
  args?: unknown;
  nonce: string;
}

export type Classified =
  | { kind: "invoke"; msg: BhqInvokeMsg }
  | { kind: "pong" }
  | { kind: "reject"; reason: string };

/**
 * Pure message triage, split out for unit tests. `sourceMatches` is the
 * caller's `event.source === iframe.contentWindow` check (a Window can't
 * cross a structured-clone boundary, so the boolean comes in from outside).
 */
export function classifyPluginMessage(
  data: unknown,
  origin: string,
  sourceMatches: boolean,
  expected: { origins: string[]; nonce: string },
): Classified {
  if (!sourceMatches) return { kind: "reject", reason: "source is not the plugin iframe" };
  if (!expected.origins.includes(origin)) {
    return { kind: "reject", reason: `unexpected origin ${origin}` };
  }
  if (typeof data !== "object" || data === null) {
    return { kind: "reject", reason: "non-object message" };
  }
  const m = data as Record<string, unknown>;
  if (m.nonce !== expected.nonce) return { kind: "reject", reason: "nonce mismatch" };
  if (m.type === "bhq:pong") return { kind: "pong" };
  if (m.type === "bhq:invoke" && typeof m.id === "string" && typeof m.cmd === "string") {
    return { kind: "invoke", msg: m as unknown as BhqInvokeMsg };
  }
  return { kind: "reject", reason: "unknown message shape" };
}

/**
 * Attach the RPC + heartbeat channel to a mounted plugin iframe. Returns
 * the teardown fn (remove listener, stop pings, clear any mid-flight ping
 * so a clean unmount isn't scored as a heartbeat miss).
 */
export function mountPluginBridge(opts: {
  iframe: HTMLIFrameElement;
  pluginId: string;
  nonce: string;
  form: SchemeForm;
}): () => void {
  const { iframe, pluginId, nonce, form } = opts;
  const origins = expectedOrigins(form, pluginId);

  // Replies target the iframe's contentWindow directly; "*" is required
  // because an opaque ("null") origin can't be named as a targetOrigin.
  // Nothing secret rides a reply that the requester didn't already prove
  // it could ask for (nonce-checked request, Rust-checked grant).
  const post = (msg: unknown) => iframe.contentWindow?.postMessage(msg, "*");

  const onMessage = (event: MessageEvent) => {
    const verdict = classifyPluginMessage(
      event.data,
      event.origin,
      event.source === iframe.contentWindow,
      { origins, nonce },
    );
    if (verdict.kind === "reject") return; // not ours (or forged) — ignore
    if (verdict.kind === "pong") {
      void commands.pluginNotePong(pluginId);
      return;
    }
    const { id, cmd, args } = verdict.msg;
    const argsJson = args === undefined ? null : JSON.stringify(args);
    commands
      .pluginInvokeProxy(pluginId, cmd, argsJson)
      .then((result) => {
        if (result.status === "ok") {
          post({ type: "bhq:result", id, ok: true, data: JSON.parse(result.data) });
        } else {
          post({
            type: "bhq:result",
            id,
            ok: false,
            error: `${result.error.kind}: ${result.error.message}`,
          });
        }
      })
      .catch((err) => {
        post({ type: "bhq:result", id, ok: false, error: String(err) });
      });
  };

  window.addEventListener("message", onMessage);
  const pingTimer = window.setInterval(() => {
    void commands.pluginNotePing(pluginId);
    post({ type: "bhq:ping" });
  }, PING_INTERVAL_MS);

  return () => {
    window.removeEventListener("message", onMessage);
    window.clearInterval(pingTimer);
    // Clean unmount: clear any mid-flight ping so the sweep doesn't score
    // a closed panel as a miss (see plugin_note_pong's doc).
    void commands.pluginNotePong(pluginId);
  };
}
