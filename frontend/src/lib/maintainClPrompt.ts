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

**Investigate** — FIRST triage the proposal queue: \`cl_list_proposals(project="${project}", status="open")\`. Open proposals are other sessions' pending edits; your direct writes will flag them as stale (their base snapshot stops matching), so read them before you touch anything and fold the worthy ones into your own edits. Then \`cl_index_search(project="${project}")\` + \`cl_folder_search\`, read \`conventions.md\` / \`notes.md\` / \`decisions.md\` and any map or audit docs, and sample the actual repo (and any in-repo ARCHITECTURE/README) to ground-truth the CL against reality. Write a findings doc listing, concretely: a verdict per open proposal (worth folding in / recommend approve as-is / recommend reject, one line of reasoning each), stale or wrong entries, thin file descriptions (ones that don't say what's inside), missing map coverage for important subsystems, undocumented gotchas/conventions visible in the code, and anything bloated to prune.

**Plan** — list the specific edits: which descriptions to sharpen, which map entries to add, which stale lines to prune, which gotchas to capture, which files to remove outright. Fold the content of worthwhile open proposals into these edits rather than leaving it stranded in the queue. Every addition is a high-signal one-liner; bias the net size change toward neutral-or-smaller. Surface the plan in chat.

**Apply** — edit the CL files under \`${project}\`'s CL directory via \`Write\`, then \`cl_rescan(project="${project}")\` so the index reflects the changes. As a dispatched maintenance session you are authorized to edit directly (a normal session files \`cl_propose\` instead — including \`delete\` proposals, which the user can approve from the queue). To remove a file, delete it via \`Bash\` and rescan. Improve, don't destroy; \`decisions.md\` is append-only (never rewrite history); preserve the user's voice; one-liners over paragraphs.

**Verify** — re-read the changed CL; confirm it's accurate, no larger than before, and that a cold reader could find the right files from it. Re-run \`cl_index_search\` to confirm the index synced, and re-run \`cl_list_proposals(project="${project}", status="open")\` — anything still open should be deliberate.

**Boundaries:** edit only \`${project}\`'s CL (read its repo to verify — don't change code). Don't commit to the project repo (the CL lives outside it) and don't push. You cannot approve or reject proposals yourself — resolution is host-only. When you're done, summarize what you added and pruned, AND end with a per-proposal recommendation list (approve / reject + one line why) so the user can clear the queue from the review UI — proposals you folded into direct edits should be listed as "reject (superseded by this maintenance pass)"; the queue's conflict detection marks them stale, so nothing is lost if the user approves one anyway. Then ask to close.`;
}
