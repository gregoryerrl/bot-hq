/**
 * The `@font-face` sources `index.css` declares, and where each must live.
 *
 * **None of the three Industrial Terminal fonts had ever loaded in a built
 * app** (found in review, round 7, 2026-08-17): `index.css` wrote
 * `url("./fonts/…")` — relative to the CSS file — while the files live in
 * `public/fonts/`, which Vite copies to the dist ROOT. Vite emits an
 * unresolvable URL verbatim, so the built stylesheet at `dist/assets/…css`
 * asked for `dist/assets/fonts/…` and got a 404; Hanken Grotesk, Inter and
 * JetBrains Mono — the last of which renders the whole chat thread — all fell
 * back to generic families. No gate could see it: cargo, vitest and tsc pass
 * either way, `npm run build` succeeds, and the webview screenshot tool does
 * not render in the agent environment. On macOS the mono fallback reads as
 * "fine", which is how it survived.
 *
 * The rule this encodes: a font is referenced by its ROOT-ABSOLUTE public
 * path (`/fonts/x`), which Vite rewrites relative to the built CSS under the
 * relative `base` (`url(../fonts/x)` — measured), and the file it names must
 * exist under `public/`. `fonts.test.ts` reads the real `index.css` and the
 * real `public/` directory through these two functions.
 */

/** Every `url("…")` inside a `@font-face` block, in declaration order. */
export function fontFaceUrls(css: string): string[] {
  const out: string[] = [];
  const blocks = css.match(/@font-face\s*\{[^}]*\}/g) ?? [];
  for (const block of blocks) {
    for (const m of block.matchAll(/url\(\s*["']?([^"')]+)["']?\s*\)/g)) {
      out.push(m[1]);
    }
  }
  return out;
}

/**
 * The public-directory-relative path a font URL must resolve to, or `null`
 * when the URL is not a root-absolute public reference — a relative `./x` or
 * `../x` is exactly the shape that never loaded, and an `http(s):` URL is a
 * network dependency the app deliberately does not have.
 */
export function publicPathOf(url: string): string | null {
  if (!url.startsWith("/") || url.startsWith("//")) return null;
  return url.slice(1);
}
