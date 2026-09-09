/**
 * The app declares a dark color scheme, so the controls this stylesheet does
 * NOT draw are dark too. See `theme.ts` for the Fedora/KDE incident.
 *
 * Reads the REAL `index.css` for the reason `fonts.test.ts` gives: a fixture
 * would pass with the production file broken, which is the exact shape this
 * test exists to close. `import.meta.glob` rather than `node:fs` because this
 * tsconfig carries no `@types/node` — `readFileSync` fails `tsc --noEmit`,
 * which is a gate, not a preference.
 */
import { describe, expect, it } from "vitest";
import { rootColorScheme } from "./theme";

const INDEX_CSS = import.meta.glob("../index.css", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

describe("index.css color scheme", () => {
  const css = Object.values(INDEX_CSS)[0] ?? "";

  it("found index.css", () => {
    expect(Object.keys(INDEX_CSS)).toEqual(["../index.css"]);
    expect(css.length).toBeGreaterThan(0);
  });

  it("declares color-scheme: dark on :root", () => {
    expect(rootColorScheme(css)).toBe("dark");
  });

  it("never hands the choice back to the host theme", () => {
    // `light dark` or `normal` would restore the Fedora behaviour: the host
    // GTK theme decides, and on a KDE box the AppImage forces Adwaita:light.
    expect(rootColorScheme(css)).not.toMatch(/light|normal/);
  });
});

describe("rootColorScheme", () => {
  it("returns null when :root declares no scheme", () => {
    expect(rootColorScheme(":root { color: red; }")).toBeNull();
  });

  it("ignores a color-scheme outside :root", () => {
    expect(rootColorScheme("body { color-scheme: light; }")).toBeNull();
  });

  it("ignores a commented-out declaration", () => {
    expect(rootColorScheme(":root { /* color-scheme: dark; */ }")).toBeNull();
    expect(rootColorScheme(":root { /* nothing */ color-scheme: dark; }")).toBe("dark");
  });

  it("does not accept a custom property or a vendor prefix as the real thing", () => {
    // A pin that `--brand-color-scheme` or `-webkit-color-scheme` satisfies
    // would stay green with the actual declaration missing.
    expect(rootColorScheme(":root { --brand-color-scheme: dark; }")).toBeNull();
    expect(rootColorScheme(":root { -webkit-color-scheme: dark; }")).toBeNull();
  });

  it("takes the last declaration, which is what the cascade resolves to", () => {
    expect(rootColorScheme(":root { color-scheme: light; color-scheme: dark; }")).toBe("dark");
  });
});
