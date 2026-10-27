// Shared by every `webview_*` snippet: the context the action would land in
// and the guards that must all pass BEFORE any effect. Runs in the bot-hq
// window (Tauri `eval_with_callback`) and, byte-identical, in the frontend's
// jsdom test fixture — keep it free of app globals.
//
// `args.expected_route`: a path-segment prefix of `location.pathname`
//   (`/sessions/s1` matches `/sessions/s1` and `/sessions/s1/x`, never
//   `/sessions/s10`). `args.expected_session_id`: the exact `/sessions/<id>`
//   segment. `args.lease`: `{gen, until_ms}` as admitted by the host — `gen`
//   must still equal the generation the host mirrored into
//   `window.__bhq_lease_gen` (0 when nothing was ever mirrored), and, for a
//   lease HOLDER (`until_ms` non-null), the wall clock (`Date.now()`, Unix
//   epoch milliseconds, the same clock the host stamped) must not have passed
//   the deadline at the moment of actuation.
((args) => {
  const route = location.pathname;
  const m = /^\/sessions\/([^/]+)/.exec(route);
  const session_id = m ? decodeURIComponent(m[1]) : null;
  const base = { route, session_id };
  const refuse = (reason, detail) =>
    Object.assign({ outcome: "refused", ok: false, matched: false, reason }, base, detail === undefined ? {} : { detail });
  const lease = args.lease || null;
  if (lease) {
    const mirror = typeof window.__bhq_lease_gen === "number" ? window.__bhq_lease_gen : 0;
    if (mirror !== lease.gen) return refuse("lease_lost", { admitted_gen: lease.gen, current_gen: mirror });
    if (lease.until_ms !== null && lease.until_ms !== undefined && Date.now() > lease.until_ms) {
      return refuse("lease_expired", { until_ms: lease.until_ms, now_ms: Date.now() });
    }
  }
  if (args.expected_route !== undefined && args.expected_route !== null) {
    const want = args.expected_route.replace(/\/+$/, "") || "/";
    const here = route.replace(/\/+$/, "") || "/";
    const ok = want === "/" ? true : here === want || here.startsWith(want + "/");
    if (!ok) return refuse("route", { expected_route: args.expected_route });
  }
  if (args.expected_session_id !== undefined && args.expected_session_id !== null && session_id !== args.expected_session_id) {
    return refuse("session", { expected_session_id: args.expected_session_id });
  }
  let el = null;
  if (args.selector !== undefined && args.selector !== null) {
    try {
      el = document.querySelector(args.selector);
    } catch (e) {
      return refuse("invalid_selector", { selector: args.selector, error: String(e && e.message || e) });
    }
    if (!el) return refuse("no_match", { selector: args.selector });
  }
  return { base, el, refuse };
})
