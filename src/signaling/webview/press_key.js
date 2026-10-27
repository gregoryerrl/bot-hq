// keydown + keypress + keyup for `key` on the element matching `selector`;
// without a selector, on the active element (or the document). A selector
// that matches nothing is refused — it never falls back to whatever happens
// to have focus.
((args) => {
  const g = GUARD(args);
  if (g.outcome) return g;
  const target = g.el || document.activeElement || document;
  for (const type of ["keydown", "keypress", "keyup"]) {
    target.dispatchEvent(new KeyboardEvent(type, { key: args.key, bubbles: true, cancelable: true }));
  }
  const name = target === document ? "document" : (target.tagName || "").toLowerCase();
  return Object.assign({ outcome: "done", ok: true, matched: true, target: g.el ? "selector" : "active_element", target_tag: name }, g.base);
})
