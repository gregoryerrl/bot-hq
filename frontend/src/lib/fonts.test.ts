/**
 * The fonts `index.css` declares actually exist where the built app will look
 * for them. See `fonts.ts` for the incident: three `@font-face` sources with a
 * CSS-relative `./fonts/…` URL, files under `public/fonts/`, and no gate that
 * could tell — the built stylesheet 404'd every font since the Industrial
 * Terminal migration.
 *
 * Reads the REAL `index.css` and the REAL `public/` directory rather than a
 * fixture, because a fixture would pass with the production file broken —
 * the exact shape this test exists to close.
 */
import { describe, expect, it } from "vitest";
import { fontFaceUrls, publicPathOf } from "./fonts";

/**
 * `import.meta.glob` rather than `node:fs`, as `overflow.test.ts` and
 * `framing.test.ts` explain: this tsconfig carries no `@types/node`, so
 * `readFileSync`/`__dirname` fail `tsc --noEmit` — a gate, not a preference.
 * The CSS is imported raw; the public files are only ENUMERATED (a lazy glob
 * yields its matched paths at transform time and imports nothing).
 */
const INDEX_CSS = import.meta.glob("../index.css", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;
const PUBLIC_FILES = Object.keys(import.meta.glob("../../public/**/*"));
const PUBLIC_PREFIX = "../../public/";

describe("index.css font sources", () => {
  const css = Object.values(INDEX_CSS)[0] ?? "";
  const urls = fontFaceUrls(css);

  it("found index.css", () => {
    expect(Object.keys(INDEX_CSS)).toEqual(["../index.css"]);
    expect(css.length).toBeGreaterThan(0);
  });

  it("declares the three Industrial Terminal fonts", () => {
    // Hanken Grotesk (headlines), Inter (body), JetBrains Mono (the chat
    // thread and every label-caps chip). Fewer means a face was dropped;
    // more is fine but each must still resolve.
    expect(urls.length).toBeGreaterThanOrEqual(3);
  });

  it("references every font by its root-absolute public path, and the file exists", () => {
    expect(PUBLIC_FILES.length).toBeGreaterThan(0);
    for (const url of urls) {
      const rel = publicPathOf(url);
      expect(rel, `${url} must be a root-absolute /… public path, not CSS-relative`).not.toBeNull();
      expect(
        PUBLIC_FILES,
        `${url} → public/${rel} must exist (public/ is copied to the dist root)`,
      ).toContain(PUBLIC_PREFIX + rel);
    }
  });
});

describe("fontFaceUrls / publicPathOf", () => {
  it("extracts only @font-face sources, in order", () => {
    const css = `
      .x { background: url("/img/bg.png"); }
      @font-face { font-family: "A"; src: url("/fonts/a.woff2") format("woff2"); }
      @font-face { font-family: "B"; src: url('./fonts/b.ttf'); }
    `;
    expect(fontFaceUrls(css)).toEqual(["/fonts/a.woff2", "./fonts/b.ttf"]);
  });

  it("rejects the relative and remote shapes", () => {
    expect(publicPathOf("/fonts/a.woff2")).toBe("fonts/a.woff2");
    expect(publicPathOf("./fonts/a.woff2")).toBeNull();
    expect(publicPathOf("../fonts/a.woff2")).toBeNull();
    expect(publicPathOf("https://x/y.woff2")).toBeNull();
    expect(publicPathOf("//cdn/y.woff2")).toBeNull();
  });
});
