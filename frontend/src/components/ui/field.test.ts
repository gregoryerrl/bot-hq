// @vitest-environment node
import { describe, expect, it } from "vitest";
import { fieldClass } from "./field";

const classes = (s: string) => s.split(/\s+/).filter(Boolean);

describe("the one field (DESIGN.md amendment 2: boxed)", () => {
  it("is a box with an outline that lights on focus", () => {
    const c = classes(fieldClass);
    for (const want of [
      "rounded",
      "border",
      "border-outline-variant",
      "bg-surface-container-lowest",
      "font-code-sm",
      "text-code-sm",
      "focus:border-primary",
      "focus:ring-1",
      "focus:ring-primary",
      "placeholder:text-subtle",
    ]) {
      expect(c, want).toContain(want);
    }
    // The underline-only shape it replaces: no bottom-border-only, no flat
    // radius, no transparent fill, no beige placeholder.
    for (const gone of ["border-b", "border-0", "rounded-none", "bg-transparent", "placeholder:text-on-surface-variant"]) {
      expect(c, gone).not.toContain(gone);
    }
  });
});
