/**
 * Hardcoded, engineered prompt for a dispatched CL-maintenance session.
 * Delivered as the duo's first message (see the `dispatch_session` Tauri
 * command). Self-executing: the duo opens the session and maintains the chosen
 * project's Context Library without further user input.
 *
 * Kept here (not in Rust) so it's HMR-iterable in dev and unit-testable; the
 * `dispatch_session` command stays generic — it broadcasts whatever prompt it
 * is handed.
 */
export function maintainClPrompt(project: string): string {
  return `You've been dispatched to **maintain the Context Library (CL) for \`${project}\`**. This is your whole task this session — editing \`${project}\`'s CL is the job you're authorized to do.

**Goal:** make \`${project}\`'s CL an accurate, high-signal "study notes" layer so future sessions orient from it instead of re-reading the whole codebase — and keep it LIGHTER than the codebase (prune as much as you add).

**What belongs in CL** is what the code doesn't carry — a where-things-live map (subsystem → the 2-3 files + entry points), conventions (formatter, test/build commands, commit rules, deploy gates), gotchas, decision rationale, and "why it's weird here." If \`grep\` finds it in seconds, it does NOT belong in the CL.

Work the IPAV phases:

**Investigate** — \`cl_index_search(project="${project}")\` + \`cl_folder_search\`, read \`conventions.md\` / \`notes.md\` / \`decisions.md\` and any map or audit docs, and sample the actual repo (and any in-repo ARCHITECTURE/README) to ground-truth the CL against reality. Write a findings doc listing, concretely: stale or wrong entries, thin file descriptions (ones that don't say what's inside), missing map coverage for important subsystems, undocumented gotchas/conventions visible in the code, and anything bloated to prune.

**Plan** — list the specific edits: which descriptions to sharpen, which map entries to add, which stale lines to prune, which gotchas to capture, which files to remove outright. Every addition is a high-signal one-liner; bias the net size change toward neutral-or-smaller. Surface the plan in chat.

**Apply** — write each changed file with \`cl_write_file(project="${project}", file_path, content)\` — it replaces the whole file, creates missing parents, and rescans the index automatically. To remove a file, delete it via \`Bash\` and \`cl_rescan(project="${project}")\`. Improve, don't destroy; \`decisions.md\` is append-only (never rewrite history); preserve the user's voice; one-liners over paragraphs.

**Verify** — re-read the changed CL; confirm it's accurate, no larger than before, and that a cold reader could find the right files from it. Re-run \`cl_index_search\` to confirm the index synced.

**Boundaries:** edit only \`${project}\`'s CL (read its repo to verify — don't change code). Don't commit to the project repo (the CL lives outside it) and don't push. When you're done, summarize what you added and pruned, then ask to close.`;
}
