import { useState } from "react";
import { cn } from "../lib/cn";
import { authorColorClass } from "./authorColor";

/** One pending tray row, as `list_session_tray` returns it. */
export type TrayRow = {
  id: number;
  choice_id: string;
  agent: string;
  kind: string;
  prompt: string;
  options: string[];
  status: string;
  asked_at: string;
  /** The gated command awaiting approval; null for an ordinary question. This
   *  is the discriminator between "somebody is blocked on this RIGHT NOW" and
   *  "a question is waiting whenever you get to it". */
  command_text: string | null;
};

/**
 * Is this row an approval — something synchronously blocked on a yes/no?
 *
 * **Keyed on the OPTIONS, not on `command_text`.** The first version of this
 * asked `command_text !== null`, which is set by `ask_user_choice_inner` for
 * ToolBlocklist (action-gate) rows *alone*. A parked `request_approval` — the
 * push gate, `Allow \`git push\` to \`staging\`?` — carries no command and was
 * therefore classified as an ordinary question. Counted across the archive:
 * **10 of 31 approvals**, every one of them a push gate, shown to the user as
 * something they could answer whenever they got to it while a pre-push hook
 * blocked on it.
 *
 * Both gate kinds ask exactly `Approve`/`Reject`; an `ask_user_choice` question
 * carries free-form options. Since round 8 the backend also says so at insert
 * (`kind = "approval"`, `storage::is_gate_row`), so the kind is the primary
 * signal here and the exact-menu check is the fallback for rows parked before
 * that (`kind = "choice"` with the gate menu). Measured on the live DB on
 * 2026-08-17, before the kind existed: 95 rows carried the exact menu, all of
 * them gates (78 with a command, the rest push gates), and 0 command-carrying
 * rows had any other menu — the fallback catches every legacy gate and
 * false-positives on nothing. Retire it once no such rows remain.
 */
export function isApproval(r: {
  kind?: string;
  options: readonly string[];
}): boolean {
  return (
    r.kind === "approval" ||
    (r.options.length === 2 &&
      r.options[0] === "Approve" &&
      r.options[1] === "Reject")
  );
}

/**
 * Is this row a TRAY item — an ordinary question? (rc3 D35)
 *
 * The user: *"halt is not on tray anymore, its a declared state."* A halt is
 * the banner; an approval is the gate; only a question lives in the tray. Every
 * tray surface — the list, the pill badge, the dashboard tile, the header
 * bell — counts through this, so none of them can claim "one item on tray"
 * over a tray with nothing in it.
 */
export function isTrayItem(r: {
  kind: string;
  options: readonly string[];
}): boolean {
  return r.kind !== "halt" && !isApproval(r);
}

/**
 * Above this, a halt reason gets a "show the full recap" toggle.
 *
 * **Set from what participants actually write, not from taste.** Across the 28
 * halts on record the reason averages 277 characters and the longest is 1,166 —
 * because the recap the user asked for is a recap: *"PR #514 is open, CI fully
 * green (ci 3m40s, quality 3m35s), mergeStateStatus CLEAN…"*. Rendered whole,
 * that is ~15 lines of banner sitting on top of the input box, and the banner
 * would push away the very box it exists to be adjacent to.
 *
 * Note the prompts never had to be changed to get this. The brainstorm assumed
 * the recap would need one — *"its value depends on a prompt change with no
 * gate behind it"* — and participants were already writing them.
 */
const RECAP_CLAMP_CHARS = 200;

/**
 * **Why the session has stopped, above the box where you answer it.**
 *
 * A halt is the session saying it needs the user. Until now it said so in the
 * tray — a list you switch to, alongside every question ever asked and answered
 * — while the input box sat empty with no explanation. The whole arc that
 * produced this banner was the user asking "why is it stopped?" and the answer
 * requiring six queries across two tables and a log.
 *
 * Three properties, and each one is a bug this replaces:
 *
 * 1. **One slot.** Halts used to accumulate: answering a tray card released the
 *    ring but left its row pending (rc3 D28), so the bell stayed lit, the user
 *    answered again, and the agent parked another. Measured across the archive:
 *    52 occasions where a second row opened while the first was unanswered, the
 *    worst one row sitting under six more for 53 minutes. D28 fixed the cause;
 *    this makes a recurrence visible immediately, because a stale halt would be
 *    a sentence contradicting a session you can watch working.
 *
 * 2. **It says what it is waiting FOR.** The agent knows, at the moment it
 *    stops. That knowledge used to go nowhere the user would look.
 *
 * 3. **It is adjacent to the answer.** The user's own case: a halt asking for
 *    the output of `php tinker …`, and the paste that satisfies it typed one
 *    line below. Read and answer without switching context.
 *
 * The tray keeps structured picks — a multiple-choice question is a click, not
 * a sentence. The rule is: this banner is the session's STATE, always present
 * while halted; a tray choice is an optional pick attached to it.
 */

/** The session's declared halt — SESSION state, never a tray row (rc3 D35).
 *  One slot by construction: the schema holds exactly one, so "there can never
 *  be 2 halts" stopped being a display rule and became a fact. */
export type SessionHalt = {
  declared_by: string;
  reason: string;
  declared_at: string;
};

export function HaltBanner({
  halt,
  rows,
  label,
  onOpenTray,
}: {
  /** The session's halt slot, from `get_session_halt`. Null = not halted. */
  halt?: SessionHalt | null;
  /** Every tray row for this session — read ONLY for the questions pointer.
   *  Halts are not rows (rc3 D35); approvals belong to the gate. */
  rows: readonly TrayRow[];
  /** slug → what to print for it (rc3 D10/D20), so the banner names a
   *  participant exactly as the chat byline does. */
  label?: (agent: string) => string;
  /** Jump to the tray, when a pick is waiting there too. */
  onOpenTray?: () => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const pending = rows.filter((r) => r.status === "pending");
  const approvals = pending.filter(isApproval);
  const choices = pending.filter(isTrayItem);

  // **HALT is a claim about the SESSION, not about the tray** (rc3 D32).
  //
  // The first version of this banner said HALT whenever any row was pending —
  // so parking a question printed "HALT" over a session that was still working,
  // and the status line underneath correctly said two participants were mid-turn.
  // The user: "parking a question in tray toggles the halt (it should not), its
  // asynchronous."
  //
  // They are right, and it is the semantics rather than the wording:
  // `ask_user_choice` is non-blocking BY DESIGN — the agent parks a question and
  // carries on. Only `halt` / `mark_awaiting_user` is a participant saying it has
  // stopped. So a halt row is a halt; everything else is something waiting for
  // you while the session runs.
  // Normalized, not trusted: a malformed slot (or a test mock's generic []
  // fallback) must read as "not halted" rather than crash the banner.
  const activeHalt = halt && typeof halt.reason === "string" ? halt : null;
  const halted = !!activeHalt;
  // rc3 D35: an approval owns the INPUT SLOT (the gate replaces the box and
  // the session halts on it) — a banner narrating it on top would be the
  // second surface for one fact. With no halt and no questions, render
  // nothing: a banner that is always present stops being read.
  if (!halted && choices.length === 0) return null;

  // Questions only: ONE line (user, reviewing s-761704e8: "shorten the
  // message on top of the input box, a one line 'You have questions in the
  // tray' will do"). And their clarification, verbatim: "Questions in the
  // tray is not equal to Waiting for you. Tray is asynchronous. HALT =
  // waiting for you." — so nothing here may say "waiting": the tray line is
  // an FYI, not a demand, and only the HALT path below wears that language.
  if (!halted) {
    return (
      <div
        role="status"
        aria-label="Questions in the tray"
        className="border-b border-outline-variant bg-surface-container-low px-3 py-1.5"
      >
        <button
          type="button"
          onClick={onOpenTray}
          className="text-xs text-primary underline underline-offset-2"
        >
          {`◆ ${choices.length} question${choices.length > 1 ? "s" : ""} in the tray — the session keeps working →`}
        </button>
      </div>
    );
  }

  // From here down the session is HALTED — the one state that genuinely
  // waits on the user, and the only surface allowed to say so.
  return (
    <div
      role="status"
      aria-label="Session halted"
      className="border-b border-outline-variant bg-surface-container-low px-3 py-2"
    >
      <div className="flex items-baseline gap-2">
        <span className="font-label-caps text-label-caps text-primary">
          ⏸ HALT
        </span>
        <span className="text-xs text-on-surface-variant">
          {/* rc3 D35 made "waiting on you" literal: a declared halt stops the
              ring where it stands — nobody keeps working under this header. */}
          the session is waiting on you
        </span>
      </div>
      {/* ONE halt line, because the session holds exactly one halt slot (rc3
          D35) — a later declaration replaces the earlier, so the freshest recap
          is always the one on screen. */}
      {activeHalt && (
        <div className="mt-1 text-sm text-on-surface">
          <span
            className={cn(
              "font-semibold",
              authorColorClass(
                label?.(activeHalt.declared_by) ?? activeHalt.declared_by,
              ),
            )}
          >
            {label?.(activeHalt.declared_by) ?? activeHalt.declared_by}
          </span>
          <span className="text-on-surface-variant"> — </span>
          {/* Clamped, not truncated: the full text is in the DOM and one click
              away. A recap the user cannot finish reading is the same failure
              the banner was built to fix. */}
          <span className={cn("whitespace-pre-wrap", !expanded && "line-clamp-3")}>
            {activeHalt.reason}
          </span>
          {activeHalt.reason.length > RECAP_CLAMP_CHARS && (
            <button
              type="button"
              onClick={() => setExpanded((v) => !v)}
              className="mt-0.5 text-xs text-primary underline underline-offset-2"
            >
              {expanded ? "show less" : "show the full recap"}
            </button>
          )}
        </div>
      )}
      {choices.length > 0 && (
        <button
          type="button"
          onClick={onOpenTray}
          className="mt-1 text-xs text-primary underline underline-offset-2"
        >
          {`${choices.length} question${choices.length > 1 ? "s" : ""} waiting in the tray →`}
        </button>
      )}
      <p className="mt-1 text-[0.7rem] text-on-surface-variant">
        {/* An approval has TAKEN the input box (rc3 D33), so "answer here"
            would point at a textarea that is not on screen. Both can be
            pending at once: one participant can park an approval while
            another halts. */}
        {approvals.length > 0
          ? "Answer the approval below first — something is blocked on it."
          : "Answering — here or in the tray — clears this and resumes the session."}
      </p>
    </div>
  );
}
