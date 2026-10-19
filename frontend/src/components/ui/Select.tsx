import { fieldClass } from "./field";

/**
 * The house `<select>` shape (round 10) — since the visual pass's S2c, the one
 * boxed field shape every input, select and textarea shares (`ui/field.ts`).
 * Sites with a deliberately different size (the SessionView phase select,
 * ViolationsPanel's filters) still spell their own. Round 11 removed the
 * `<Select>` wrapper component: every site applies the class to its own
 * `<select>`.
 */
export const selectClass = fieldClass;
