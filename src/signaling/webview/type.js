// Set `text` on the element matching `selector`, the React-friendly way
// (prototype value setter + input/change events). A `<select>` takes an
// option's value or its visible label; a checkbox takes true/false; anything
// else must be an input or textarea. Nothing is focused or changed until
// every check has passed.
// `done` reports the OBSERVED state after the dispatch (`value`, `checked`,
// `selected`), not the request: a browser may sanitize what was set.
((args) => {
  const g = GUARD(args);
  if (g.outcome) return g;
  const el = g.el;
  const want = args.text;
  const done = (detail) => Object.assign({ outcome: "done", ok: true, matched: true }, g.base, detail || {});
  const tag = el.tagName.toLowerCase();
  if (el.disabled === true || el.closest("fieldset:disabled")) return g.refuse("disabled", { tag });
  if (el.readOnly === true) return g.refuse("readonly", { tag });
  if (el instanceof HTMLInputElement && el.type === "file") return g.refuse("not_editable", { tag, type: "file" });
  if (el instanceof HTMLSelectElement) {
    const opts = Array.from(el.options);
    const byValue = opts.filter((o) => o.value === want);
    const byLabel = opts.filter((o) => o.textContent.trim() === want);
    const hits = byValue.length ? byValue : byLabel;
    if (hits.length !== 1) {
      return g.refuse(hits.length ? "ambiguous_option" : "no_option", { wanted: want, options: opts.map((o) => o.value + " | " + o.textContent.trim()) });
    }
    el.focus();
    const desc = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value");
    if (desc && desc.set) desc.set.call(el, hits[0].value); else el.value = hits[0].value;
    el.dispatchEvent(new Event("change", { bubbles: true }));
    return done({ selected: el.value, applied: el.value === hits[0].value });
  }
  if (el instanceof HTMLInputElement && el.type === "checkbox") {
    const on = ["true", "on", "1"], off = ["false", "off", "0"];
    if (!on.includes(want) && !off.includes(want)) return g.refuse("bad_checkbox_value", { wanted: want });
    const target = on.includes(want);
    el.focus();
    if (el.checked !== target) el.click();
    return done({ checked: el.checked, applied: el.checked === target });
  }
  if (!(el instanceof HTMLInputElement) && !(el instanceof HTMLTextAreaElement)) {
    return g.refuse("not_editable", { tag });
  }
  el.focus();
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  const desc = Object.getOwnPropertyDescriptor(proto, "value");
  if (desc && desc.set) desc.set.call(el, want); else el.value = want;
  el.dispatchEvent(new Event("input", { bubbles: true }));
  el.dispatchEvent(new Event("change", { bubbles: true }));
  return done({ length: el.value.length, applied: el.value === want });
})
