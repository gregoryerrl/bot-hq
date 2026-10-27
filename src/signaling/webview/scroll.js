// Scroll the element matching `selector`, or the window, to `y`.
((args) => {
  const g = GUARD(args);
  if (g.outcome) return g;
  if (g.el) {
    g.el.scrollTop = args.y;
    return Object.assign({ outcome: "done", ok: true, matched: true, scroll_top: g.el.scrollTop }, g.base);
  }
  window.scrollTo({ top: args.y, behavior: "auto" });
  return Object.assign({ outcome: "done", ok: true, matched: true, scroll_y: window.scrollY }, g.base);
})
