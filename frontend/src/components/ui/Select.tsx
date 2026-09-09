/**
 * The house `<select>` shape (round 10). Three panels declared their own
 * `selectClass` and had already drifted (ClaudeConfig: `py-1`, no focus ring;
 * Models/Roles: `py-1.5` + `focus:ring-1`); the majority shape is the one
 * here. Sites with a deliberately different size (the SessionView phase
 * select, ViolationsPanel's filters) still spell their own — folding those in
 * is a visual decision for a session with the app open. Round 11 removed the
 * `<Select>` wrapper component: every site applies the class to its own
 * `<select>` and nothing rendered the wrapper.
 */
export const selectClass =
  "w-full rounded border border-outline-variant bg-surface-container-lowest px-2 py-1.5 font-code-sm text-code-sm text-on-surface focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary";
