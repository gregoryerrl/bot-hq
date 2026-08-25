import "@testing-library/jest-dom/vitest";

/**
 * Web Storage for the jsdom environment, because Node 26 took the name.
 *
 * Node 26 ships a native `localStorage` global that stays `undefined` unless
 * the process was started with `--localstorage-file` ("ExperimentalWarning:
 * localStorage is not available because --localstorage-file was not
 * provided"). Vitest builds its jsdom environment by copying the jsdom
 * window's properties onto Node's `globalThis` and does NOT overwrite names
 * that already exist — so `localStorage` is skipped, and since vitest also
 * aliases `window` to `globalThis` (measured: `globalThis === window` is
 * true), `window.localStorage` is that same undefined value. There is no
 * reachable jsdom Storage to borrow.
 *
 * Measured on a fresh clone, Node v26.0.0: 43 failures — every one
 * `Cannot read properties of undefined (reading 'getItem')` — across the three
 * suites that touch storage (Providers, SessionView, ChatInput). The app's 25
 * storage references are all bare `localStorage.…`, so they resolve to the
 * shadowing global.
 *
 * A shim rather than a Node version pin: nothing in this repo runs the tests
 * in CI (`.github/workflows/` builds bundles and publishes the site — there is
 * no test job), so `release.yml`'s `node-version: 22` is what the BUNDLE is
 * built with, never a tested-against version. Pinning would encode a guarantee
 * that was never measured; this is correct on every Node and a strict no-op
 * where the global already works.
 *
 * jsdom's own Storage is not importable here: `@types/jsdom` is not a
 * dependency, so `import { JSDOM } from "jsdom"` fails `tsc --noEmit`, which
 * is a gate (`npm run lint`), not a preference.
 */
const memoryStorage = (): Storage => {
  const entries = new Map<string, string>();
  return {
    get length() {
      return entries.size;
    },
    // Storage coerces keys AND values to string, on every path — not just on
    // write. A shim that coerced only in setItem would return null for
    // `setItem(1, "a"); getItem(1)` where a browser returns "a". Unreachable
    // while `tsc` gates every caller to `string`, but a shim that contradicts
    // its own comment is how a wrong mechanism gets believed later.
    clear: () => entries.clear(),
    getItem: (key: string) => {
      const k = String(key);
      return entries.has(k) ? entries.get(k)! : null;
    },
    key: (index: number) => [...entries.keys()][index] ?? null,
    removeItem: (key: string) => {
      entries.delete(String(key));
    },
    setItem: (key: string, value: string) => {
      entries.set(String(key), String(value));
    },
  } as Storage;
};

for (const name of ["localStorage", "sessionStorage"] as const) {
  if (typeof globalThis[name] === "undefined") {
    Object.defineProperty(globalThis, name, {
      value: memoryStorage(),
      configurable: true,
      writable: true,
    });
  }
}
