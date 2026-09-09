#!/usr/bin/env node
/**
 * Fail the build on a circular import between source modules.
 *
 * # Why this runs at LAUNCH rather than in the test suite
 *
 * On 2026-08-13 a cycle between `lib/participants` and `components/authorColor`
 * shipped a blank window. `tsc --noEmit` was clean, `vite build` exited 0, and
 * all 333 frontend tests passed — Vitest resolves a module graph per test file,
 * so a cycle that only bites when Vite hoists both modules into ONE chunk never
 * appears there. The bundle emitted the `RESERVED` map before the constant it
 * keys on, and the app threw `Cannot access 'Bd' before initialization` on load.
 *
 * Every check that could have caught it was green. So the check lives on the one
 * path that always runs before the user sees the app.
 *
 * # What it does and does not prove
 *
 * It reads static `import`/`export ... from` specifiers and follows relative
 * ones. A cycle it reports is real. A cycle it misses is possible — dynamic
 * `import()`, a path alias, a re-export chain through `node_modules` — so this
 * is a floor, not a proof. Cycles are also not always fatal: two modules that
 * only exchange TYPES, or only touch each other inside function bodies, are
 * fine at runtime. This refuses them anyway, because deciding which cycle is
 * safe is exactly the judgement that produced the blank window.
 */

import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";

const ROOT = resolve(process.argv[2] ?? "src");
const EXTS = [".ts", ".tsx", ".js", ".jsx"];

/** Every source file under `dir`, recursively. */
function sources(dir) {
  const out = [];
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) out.push(...sources(path));
    else if (EXTS.some((e) => path.endsWith(e))) out.push(path);
  }
  return out;
}

/** Resolve a relative specifier to a real file, trying the usual suffixes. */
function resolveImport(fromFile, spec) {
  if (!spec.startsWith(".")) return null;
  const base = resolve(dirname(fromFile), spec);
  const candidates = [
    base,
    ...EXTS.map((e) => base + e),
    ...EXTS.map((e) => join(base, "index" + e)),
  ];
  for (const c of candidates) {
    try {
      if (statSync(c).isFile()) return c;
    } catch {
      /* not this one */
    }
  }
  return null;
}

// `import ... from "x"`, `export ... from "x"`, and bare `import "x"`.
const SPEC = /(?:^|\n)\s*(?:import|export)[\s\S]*?from\s*["']([^"']+)["']|(?:^|\n)\s*import\s*["']([^"']+)["']/g;

const graph = new Map();
for (const file of sources(ROOT)) {
  // Tests may legitimately reach across layers; they are not in the bundle.
  if (/\.test\.[jt]sx?$/.test(file)) continue;
  const src = readFileSync(file, "utf8");
  const edges = [];
  for (const m of src.matchAll(SPEC)) {
    const target = resolveImport(file, m[1] ?? m[2]);
    if (target && !/\.test\.[jt]sx?$/.test(target)) edges.push(target);
  }
  graph.set(file, edges);
}

// Depth-first search, reporting the first cycle found through each entry.
const cycles = [];
const state = new Map(); // file -> "visiting" | "done"
function walk(file, stack) {
  if (state.get(file) === "done") return;
  const at = stack.indexOf(file);
  if (at !== -1) {
    cycles.push([...stack.slice(at), file]);
    return;
  }
  stack.push(file);
  for (const next of graph.get(file) ?? []) walk(next, stack);
  stack.pop();
  state.set(file, "done");
}
for (const file of graph.keys()) walk(file, []);

if (cycles.length === 0) {
  console.log(`✓ no import cycles (${graph.size} modules)`);
  process.exit(0);
}

// De-duplicate: one cycle reached from several entry points is one cycle.
const seen = new Set();
const unique = cycles.filter((c) => {
  const key = [...c].sort().join("|");
  if (seen.has(key)) return false;
  seen.add(key);
  return true;
});

console.error(`\n✗ ${unique.length} circular import(s):\n`);
for (const cycle of unique) {
  console.error("  " + cycle.map((f) => relative(ROOT, f)).join("\n    → "));
  console.error("");
}
console.error(
  "A cycle lets the bundler emit a module's dependents before the module\n" +
    "itself, so a constant read at module scope is in its temporal dead zone\n" +
    "and the app throws on load. Break it with a leaf module both sides import\n" +
    "— see src/lib/participantNames.ts.\n",
);
process.exit(1);
