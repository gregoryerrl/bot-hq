/**
 * **The one field** (the 2026-10-19 visual pass, S2c; DESIGN.md amendment 2,
 * the user's pick: boxed fields). An `<input>`, `<select>` or `<textarea>` that
 * holds DATA — a path, a model id, a keyword, a role's instructions — is a box:
 * a 1px `outline-variant` outline on the sunken `surface-container-lowest`
 * fill, the house 2px radius, mono `code-sm` text, and the primary colour on
 * the outline plus a 1px ring when focused. Placeholders are hint text, so
 * they take `subtle` (amendment 3), not the warm beige.
 *
 * It replaced two shapes that had drifted apart: the underline-only
 * `terminalInputClass` (the spec's old "bottom border only" input, which could
 * not carry a select or a multi-line field) and the boxed `selectClass`; every
 * input, select and textarea that held either now takes this class.
 *
 * Not this class: the chat composer, which holds message PROSE (Inter, per
 * amendment 1) and paints its mention pills from a mirror that must match its
 * metrics exactly — `ui/Textarea`, left as it is.
 */
export const fieldClass =
  "w-full rounded border border-outline-variant bg-surface-container-lowest px-2 py-1.5 font-code-sm text-code-sm text-on-surface placeholder:text-subtle caret-primary transition-colors focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary disabled:cursor-not-allowed disabled:opacity-50";
