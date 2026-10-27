/**
 * The `webview_*` MCP tools run JavaScript in the bot-hq window. The snippets
 * live in `src/signaling/webview/*.js` (the Rust side includes them byte for
 * byte) and this module loads THOSE bytes for the jsdom fixture, composed
 * exactly as `signaling::webview_js::compose` composes them for Tauri's
 * `eval_with_callback`:
 *
 *     (() => { try { const GUARD = <guard.js>; return (<action.js>)(<args JSON>); }
 *              catch (e) { return { outcome: "unknown", … reason: "exception" }; } })()
 *
 * Keep the two compositions identical — the fixture's proof is about the
 * string the window actually evaluates.
 *
 * `import.meta.glob` with `?raw`, not `node:fs`: this tsconfig carries no
 * `@types/node` (see `fonts.test.ts`), and the glob imports the bytes as-is.
 */
const SNIPPETS = import.meta.glob("../../../src/signaling/webview/*.js", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

export type SnippetName = "guard" | "context" | "mirror" | "click" | "type" | "scroll" | "press_key";

export function snippetSource(name: SnippetName): string {
  const text = SNIPPETS[`../../../src/signaling/webview/${name}.js`];
  if (text === undefined) {
    throw new Error(`snippet ${name}.js not found among ${Object.keys(SNIPPETS).join(", ")}`);
  }
  return text;
}

/** The composition `signaling::webview_js::compose` produces, byte for byte. */
export function composeSnippet(action: SnippetName, args: unknown): string {
  return `(() => { try { const GUARD = ${snippetSource("guard")}; return (${snippetSource(action)})(${JSON.stringify(args)}); } catch (e) { return { outcome: "unknown", ok: false, matched: null, reason: "exception", detail: String(e && e.message || e) }; } })()`;
}
