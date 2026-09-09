/**
 * The effort model, post-no-inherit (2026-08-25).
 *
 * A participant's effort resolves in exactly two steps: the New-session
 * dialog's per-run pick, else its role's default (`per_role[slug]` in
 * claude-overrides.json), else `DEFAULT_EFFORT`. Nothing inherits from
 * `_all` or from the user's own settings.json knob any more — the spawn
 * floor (`reconcile_spawn_knobs`, `src/agents/spawn.rs`) guarantees a
 * concrete value on every spawn.
 *
 * "ultracode" is a dropdown CHOICE, not a storage value: it decomposes to
 * `{effort: "xhigh", ultracode: true}` (the valid claude-code pair — a role
 * that never receives `--settings` still spawns at a truthful xhigh), and
 * any concrete level decomposes to `{effort: level, ultracode: false}` so a
 * per-run pick clears a role-default ultracode.
 */

/** Effort levels claude-code accepts as `CLAUDE_CODE_EFFORT_LEVEL` values. */
export const EFFORT_LEVELS = ["low", "medium", "high", "xhigh", "max"] as const;

/** The dropdown choice that is not an env value. */
export const ULTRACODE = "ultracode";

/** MUST equal `DEFAULT_EFFORT` in `src/claude_config/overrides.rs` — a Rust
 *  test (`frontend_default_effort_matches_the_rust_floor`) pins the pair. */
export const DEFAULT_EFFORT = "medium";

/** The stored pair every surface reads/writes. */
export type EffortFields = {
  effort?: string | null;
  ultracode?: boolean | null;
};

/**
 * The dropdown choice a stored pair displays as: `"ultracode"` when the flag
 * is set (the stored effort is the implied xhigh — the flag is the choice),
 * else the stored level, else `null` (caller decides between "Default (…)"
 * and the DEFAULT_EFFORT fallback).
 */
export function effortChoiceOf(fields: EffortFields | undefined): string | null {
  if (!fields) return null;
  if (fields.ultracode === true) return ULTRACODE;
  return fields.effort ?? null;
}

/**
 * What a role's participants spawn with when the dialog row stays on Default —
 * mirrors the spawn floor, so display and resolution cannot disagree.
 */
export function roleDefaultEffort(fields: EffortFields | undefined): string {
  return effortChoiceOf(fields) ?? DEFAULT_EFFORT;
}

/**
 * A dropdown pick, decomposed to the stored pair. `""` (the dialog's Default
 * option) stores the absence pair so spawn resolves the role default; the
 * Roles tab never passes `""` (it has no Default option).
 */
export function pickToFields(value: string): {
  effort: string | null;
  ultracode: boolean | null;
} {
  if (value === "") return { effort: null, ultracode: null };
  if (value === ULTRACODE) return { effort: "xhigh", ultracode: true };
  return { effort: value, ultracode: false };
}
