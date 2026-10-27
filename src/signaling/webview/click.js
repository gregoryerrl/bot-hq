// Click the first element matching `selector`, after every guard passed.
// A disabled control swallows the click, and an element without `click()`
// (SVG) cannot take one: both are refused before anything happens.
((args) => {
  const g = GUARD(args);
  if (g.outcome) return g;
  const el = g.el;
  if (typeof el.click !== "function") return g.refuse("not_clickable", { tag: el.tagName.toLowerCase() });
  if (el.disabled === true || el.closest("fieldset:disabled")) return g.refuse("disabled", { tag: el.tagName.toLowerCase() });
  el.click();
  return Object.assign({ outcome: "done", ok: true, matched: true, tag: el.tagName.toLowerCase() }, g.base);
})
