// The context read: route and session only, no guards, no effect.
((args) => {
  const route = location.pathname;
  const m = /^\/sessions\/([^/]+)/.exec(route);
  return { outcome: "done", ok: true, matched: true, route, session_id: m ? decodeURIComponent(m[1]) : null,
    lease_gen: typeof window.__bhq_lease_gen === "number" ? window.__bhq_lease_gen : 0 };
})
