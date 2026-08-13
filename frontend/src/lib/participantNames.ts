/**
 * Display constants shared by `lib/participants` and `components/authorColor`.
 *
 * # Why this file exists
 *
 * **To break an import cycle that a passing test suite could not see.**
 * `authorColor` needs `UNKNOWN_PARTICIPANT` (it is a RESERVED label with its own
 * neutral tone), and `lib/participants` needs the colour palette (rc3 D20's
 * rotation assigns a hue per roster slot). Importing across that pair in both
 * directions is a cycle, and Vite hoists the two modules into one chunk in an
 * order the cycle does not determine — so the `RESERVED` map was emitted BEFORE
 * the constant it keys on, and the bundle threw
 * `Cannot access 'Bd' before initialization` on load. A blank window, from a
 * build that compiled cleanly and a suite of 333 green tests: Vitest resolves
 * each module graph per file, so the cycle never bit there.
 *
 * A leaf module both sides import has no cycle to order. `lib/participants`
 * re-exports the constant, so every existing importer is unchanged.
 */

/**
 * What an author the roster cannot place is called.
 *
 * Still ATTRIBUTED, never named: a message whose author no longer matches a
 * roster row (a participant removed, a legacy row from before the rename) reads
 * as this rather than falling back to the internal slug, which is what put
 * "brian" back on screen after rc3 D10 retired it.
 */
export const UNKNOWN_PARTICIPANT = "Unknown participant";
