# Phase G v1 Slice 2 Design

Status: locked + shipping | Owner: Brian (HANDS) | Reviewer: Rain (EYES, per-commit diff-gate) | Greenlight: user @ msg 3145 (blanket proceed on R1/R2/R3 refined shape)

## 1. Goal

Land the SNAP-typed protocol persistence layer drafted in the original Phase G v1 design (Stage 2 / B). SNAP footers — the 4-line `Branches/Agents/Pending/Next` block hardened during slice 1.5 (msg 3096) — graduate from a freeform writer convention into a typed, parseable, queryable structure with on-insert extraction.

Slice scope is intentionally light: typed struct + storage column + send-path hook + this design doc. No DB tables, no normalized rows, no historical backfill. Query workload has not yet justified more.

**Slice 2 surface (Code):**
- C1 — `internal/snap` package: `SNAP` struct, `Format()`, `Parse()` w/ paren-aware tokenizer, round-trip + regression tests. Commit `cf2c4a2`.
- C2 — `messages.snap_json TEXT NOT NULL DEFAULT ''` column via guarded ALTER, generalizing the rebuild_gen helper to `addColumnIfMissing(table, column, decl)`. Commit `0625d5c`.
- C3 — `extractSnapJSON()` send-path hook on `InsertMessage`, parse-error policy log+warn+empty, plus a fold of all 4 of Rain's C1/C2 diff-gate observations (Format() doc-comment, CRLF tolerance, splitDepth0 malformed contract, SQL identifier guard). Commit `e29cae9`.
- C4 — this doc + `errors.Is` durability fix + bad-identifier guard test (Rain C3 obs #1 + #3 fold).

**Out of scope for slice 2:**
- Normalized rows / `snap_entries` table — deferred to slice 3+ if query workload justifies the cost.
- Backfill of historical SNAPs — forward-only on the wire; if needed, an isolated one-shot script (not a migration).
- Sub-shape parsing of list items (`branch@sha (state)`, `id(state)`) — opaque strings in v1.
- Quoting / backslash escape mechanisms — paren-depth is v1's sole escape; v2+ if format strain emerges.
- Rate-limiting on parse-error log spam — accept unbounded; revisit if a flood materializes.

## 2. Protocol design — `internal/snap`

### 2.1 Canonical wire form

```
SNAP:
Branches: <repo:branch@sha(state)>, ...
Agents:   <id(state)>, ...
Pending:  <blocker>
Next:     <action>
```

Marker line `SNAP:` (trimmed). Fixed-order fields. Format emits canonical 10-character label-plus-padding alignment. Parse is whitespace-tolerant on the inter-label spacing.

The writer convention is documented in user memory `feedback_snap_footer.md`; this doc is the canonical machine-side spec. If the two ever diverge, this doc wins for parser behavior and the memory file is updated.

### 2.2 Typed struct

```go
type SNAP struct {
    Branches []string `json:"branches"`
    Agents   []string `json:"agents"`
    Pending  string   `json:"pending"`
    Next     string   `json:"next"`
}
```

List items (Branches, Agents) are stored as opaque strings. Sub-shape (`branch@sha (state)`, `id(state)`) is NOT normalized in v1. Defer to slice 3+ if query workload demands.

### 2.3 Escape mechanism — paren-depth, v1 sole

Commas inside parentheses are part of the surrounding list item, not a separator. Implementation: `splitDepth0` walks the field, increments depth on `(`, decrements on `)` (guarded against underflow), splits on `,` only at depth 0.

**Rationale.** SNAPs in practice contain commas inside paren-delimited notes. Concrete example surfaced by hub msg 3122: `bot-hq:main@9b17042 (slice 1.5 + followup live, awaiting rebuild to render border)`. Naive split would fragment this into two items. Locked as a regression fixture in `TestRegressionMsg3122`.

**Non-goal: no quoting, no backslash.** v1 has no escape syntax beyond paren-depth. If format strain emerges (commas in non-paren contexts, paren-as-literal), v2+ adds explicit quoting. Do NOT backdoor it into v1 — keeps the parser simple and bounds spec drift.

**Malformed input contract.** Unclosed `(` keeps depth>0 → entire string returns as one item. Unmatched `)` is guarded against underflow → splits at outer `,` as if the rogue `)` were literal. Both cases produce no panic. Locked in `TestSplitDepth0Malformed`.

### 2.4 Parse contract

Strict-positional: 4 labeled lines must follow the SNAP: marker in fixed order (Branches, Agents, Pending, Next). Anything before the marker is preamble and ignored. Errors:

- `ErrNoSNAPBlock` — no `SNAP:` marker line found. Sentinel; common case for non-orchestrator messages.
- `ErrMalformedFields` — block found but truncated, out-of-order, or wrong labels.

### 2.5 Round-trip discipline

`Parse(Format(s)) == s` is required and asserted across a fixture set incl. all-empty.

`Format(Parse(raw)) == raw` is aspirational only and **not asserted** — raw input may have whitespace / alignment variance that Format normalizes.

## 3. Storage — `messages.snap_json`

A single TEXT column on the existing `messages` table. Default `''` empty string for messages with no SNAP, no-block parse error, or other parse errors.

```sql
ALTER TABLE messages ADD COLUMN snap_json TEXT NOT NULL DEFAULT '';
```

Applied via `addColumnIfMissing("messages", "snap_json", "TEXT NOT NULL DEFAULT ''")` — the same idempotent guarded-ALTER pattern that landed `agents.rebuild_gen` (Phase G v1 #20). The helper was generalized in slice 2 C2 to take a table parameter; identifier guard added in C3 (`^[A-Za-z_][A-Za-z0-9_]*$`) closes the SQL-injection surface.

**No table.** Originally proposed as `snap_entries` joined to `messages`. Rain pushed back pre-dispatch (msg 3133): query workload isn't real, ~50 SNAPs in hub, schema-first is overengineering. JSON-on-messages serves until query patterns demand more.

## 4. Send-path hook — `extractSnapJSON`

Single integration point on `InsertMessage`:

```go
snapJSON := extractSnapJSON(msg.FromAgent, msg.Content)
// included in INSERT
```

The helper parses the message content, marshals to canonical JSON, returns the JSON string. Empty string return is the universal "not available" signal.

### 4.1 Parse-error policy — log+warn+empty

Per Rain pre-dispatch refinement #1 (msg 3137):

| Outcome | Action |
|---|---|
| `ErrNoSNAPBlock` (no marker) | Silent return `""` — common case, not drift |
| Other parse errors | `log.Printf("[snap] warn: parse failed for message from %s: %v", ...)` + return `""` |
| Marshal error | `log.Printf("[snap] warn: marshal failed for message from %s: %v", ...)` + return `""` |

**Insert never fails on a SNAP error.** The message content is still useful even if its footer can't be parsed — only the structured form is unavailable.

**Sentinel comparison via `errors.Is`** (per Rain C3 obs #1, msg 3155). `Parse` currently returns `ErrNoSNAPBlock` directly so `==` would work, but a future wrapping (`fmt.Errorf("%w", ErrNoSNAPBlock)`) would silently break and start spamming warns on every non-SNAP message. Cheap durability fix already folded in C4.

### 4.2 Log spam trade-off

The log is unbounded. In normal operation only orchestrator messages have footers, drift is rare, and `[snap] warn:` lines are diagnostic. If a flood materializes (e.g. an agent emits malformed SNAPs continuously), rate-limiting can be added — until then, accept the trade-off. Worth knowing the contract.

## 5. JSON wire shape and query contract

Canonical:

```json
{"branches":["bot-hq:main@abc"],"agents":["brian(idle)"],"pending":"none","next":"ship"}
```

### 5.1 Null-on-empty arrays

Per Rain C3 obs #2 (msg 3155). `splitDepth0("")` returns nil; `json.Marshal(nil []string)` emits `null`. So an all-empty SNAP serializes to:

```json
{"branches":null,"agents":null,"pending":"","next":""}
```

**This is the accepted wire shape.** Downstream consumers using `json_extract(snap_json, '$.branches[0]')` must handle null on empty arrays. We deliberately do NOT coerce to `[]string{}` in Parse — coercion adds a layer for a query-time concern that hasn't materialized.

### 5.2 Empty `snap_json`

Empty-string `snap_json` (no block, no-block parse error, malformed parse error) is distinct from a populated SNAP with empty arrays. Filters should distinguish:

```sql
-- messages with any SNAP at all
WHERE snap_json != ''
-- messages whose SNAP names a particular agent
WHERE snap_json LIKE '%"brian(%'
```

## 6. Known limitations

- **Multi-SNAP per message → first occurrence wins.** Per Rain C1 obs #2. A reply that quotes a prior agent's SNAP block (e.g. while reviewing it) would have its outer SNAP shadowed by the inner. Out of v1 scope per the writer convention (one footer per substantive reply); pinned here as a known limitation.
- **CRLF tolerance is implicit.** `strings.TrimSpace` strips `\r`, so `\r\n` line endings happen to parse correctly. Locked in `TestParseCRLFTolerant`. Not a runtime concern (hub is LF-only) but defensive against pasted/converted content.
- **Identifier guard is unreachable.** `addColumnIfMissing` validates `table`/`column` against a regex; all current callers pass compile-time literals. Guard exists to fail fast on a future contributor who introduces dynamic input. Test `TestAddColumnIfMissingRejectsBadIdentifier` locks the contract regardless.
- **Round-trip via Format → Parse round-trips unstructured raw → canonical raw, lossily.** Whitespace variance, label-padding differences, and trailing-newline omission all collapse in canonical form. By design.

## 7. Sequencing and post-merge

- **Branch:** `brian/phase-g-v1-slice-2`
- **Commits:** C1 `cf2c4a2`, C2 `0625d5c`, C3 `e29cae9`, C4 (this doc + small folds)
- **Diff-gate:** Rain per-commit. C1 PASS msg 3149, C2 PASS msg 3151, C3 PASS msg 3155.
- **Merge:** ff to main on Rain C4 review-ack. No rebuild needed (no UI surface; storage-only addition that activates on next agent message).
- **Rebuild #12:** optional for hot-reload of the live hub binary; existing in-flight messages don't backfill (forward-only, by design).

## 8. References

- Phase G v1 arc: `docs/arcs/phase-g-v1.md`
- Phase G v1 design (master): `docs/plans/2026-04-26-phase-g-v1-design.md`
- Slice 1.5 design (predecessor): `docs/plans/2026-04-26-phase-g-v1-slice-1.5-design.md`
- Writer convention (user memory): `feedback_snap_footer.md`
- Sentinel SNAP regression case (msg 3122 origin): `internal/snap/snap_test.go::TestRegressionMsg3122`
- Migration precedent (rebuild_gen, Phase G v1 #20): `internal/hub/db.go::addColumnIfMissing` + `TestRebuildGenMigrationIdempotent`
