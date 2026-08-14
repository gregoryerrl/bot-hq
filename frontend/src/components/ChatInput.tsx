import { useEffect, useRef, useState, type FormEvent } from "react";
import { Button } from "./ui/Button";
import { Textarea } from "./ui/Textarea";
import { ErrorBanner } from "./ErrorBanner";
import { errorMessage } from "../hooks/useInvoke";
import { cn } from "../lib/cn";
import { authorColorClass } from "./authorColor";
import { UNKNOWN_PARTICIPANT } from "../lib/participants";
import { anyBusy, isLocked, type AgentBusy, type SessionActivity } from "../stores/activity";

/** One participant the `@` picker can insert (rc3 D17). */
export type Mentionable = {
  /** What goes in the text, after the `@`. The backend parses this. */
  slug: string;
  /** What the user reads — the display rule's `ROLE · Model`. */
  label: string;
};

/**
 * The `@`-token the caret is sitting in, or `null`.
 *
 * Walks BACK from the caret to the nearest `@`, giving up at whitespace — so
 * `@adv|` is a live token and `@adv thoughts|` is not, which is what makes the
 * picker close by itself once the user moves on.
 *
 * The boundary rule matches the backend parser (`core::mentions`) rather than
 * approximating it: an `@` preceded by an alphanumeric is part of an email
 * address, not a mention, and offering a picker there would suggest bot-hq is
 * about to do something it will not.
 */
export function activeMention(
  text: string,
  caret: number,
): { start: number; query: string } | null {
  let i = caret;
  while (i > 0) {
    const ch = text[i - 1];
    if (ch === "@") {
      const before = i >= 2 ? text[i - 2] : undefined;
      if (before !== undefined && /[a-zA-Z0-9]/.test(before)) return null;
      return { start: i - 1, query: text.slice(i, caret) };
    }
    if (/\s/.test(ch)) return null;
    i -= 1;
  }
  return null;
}

/**
 * Participants whose slug, or any WORD of whose label, starts with what has
 * been typed so far.
 *
 * **Word-prefix rather than substring**, which is not a detail: the label
 * carries the model (`EYES · Claude Opus 5`), so a substring match on `@e`
 * would offer every participant running Claud**e** — and the first of them,
 * not the one whose slug is `eyes`, is what Enter would then insert. Matching
 * the label at all is still worth it, because the user reads `EYES` and
 * `DeepSeek`, never the slug.
 */
export function matchMentionables(
  all: Mentionable[],
  query: string,
): Mentionable[] {
  const q = query.toLowerCase();
  if (!q) return all;
  return all.filter((m) => {
    if (m.slug.toLowerCase().startsWith(q)) return true;
    return m.label
      .toLowerCase()
      .split(/[^a-z0-9]+/)
      .some((word) => word.startsWith(q));
  });
}

interface ChatInputProps {
  placeholder?: string;
  onSend: (text: string) => Promise<void> | void;
  disabled?: boolean;
  /**
   * This session's participants, for the `@` picker (rc3 **D17**).
   *
   * **A picker rather than free text, and that is the design rather than a
   * nicety**: mentioning somebody who is not in the session becomes impossible
   * to EXPRESS, instead of an error to detect and report. Left undefined the
   * textarea behaves exactly as before — typing `@` opens nothing — which is
   * what every surface that is not a session chat wants.
   */
  mentionables?: Mentionable[];
  /**
   * The session's activity. While `busy`/`cancelling` the textarea is
   * REPLACED by a turn-status line (which participants are working) + Stop —
   * the user stops the turn to reclaim the input, then types. `idle` /
   * `awaiting_user` show the normal textarea + Send.
   */
  activity?: SessionActivity;
  /** Per-participant busy flags, for the turn-status line. The collapsed
   *  `activity` says "someone is busy"; this says which participants. */
  busy?: AgentBusy;
  /** rc3 D34: how many tray picks are staged to travel with this Send. When
   *  > 0, Send is enabled with an empty box — answering without commentary is
   *  a complete response — and a chip says what rides along. */
  stagedAnswers?: number;
  /** Busy-map key -> what to PRINT for it (rc3 D10: `ROLE · Model`, never an
   *  agent name). `SessionView` resolves it through the session's roster.
   *  Without it the status line has no roster to consult and says so, rather
   *  than printing the internal key it happens to hold. */
  busyLabel?: (key: string) => string;
  /** Roster slot -> hue (rc3 D20), so two participants of one role are told
   *  apart by colour as well as by their ordinal. */
  authorHues?: Record<string, string>;
  /** Pause the in-flight turn (the Stop button — interrupts the agents and
   *  lands the session in `paused`). Without it a locked session shows the
   *  status line but no Stop. */
  onCancel?: () => Promise<void> | void;
  /** Resume a paused session (the paused bar's Resume button). The backend
   *  releases the latch, nudges the agents, and flushes anything held. */
  onResume?: () => Promise<void> | void;
  /** Open the force-close flow from the paused bar (the parent owns the
   *  confirm dialog — same flow as the header ✕). */
  onClose?: () => void;
  /**
   * localStorage key for draft persistence. When set, the in-progress text
   * survives unmounts (navigating to another session / app restart): seeded
   * on mount, written through on change, cleared on successful send. The
   * parent must remount this component when the key changes (`key={...}`) —
   * the seed is a lazy initializer, not an effect.
   */
  draftKey?: string;
}

export function ChatInput({
  placeholder,
  onSend,
  disabled,
  mentionables,
  activity,
  busy,
  stagedAnswers = 0,
  busyLabel,
  authorHues,
  onCancel,
  onResume,
  onClose,
  draftKey,
}: ChatInputProps) {
  const [value, setValue] = useState(() =>
    draftKey ? (localStorage.getItem(draftKey) ?? "") : "",
  );
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [cancelling, setCancelling] = useState(false);
  const [resuming, setResuming] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  // Where the caret is, tracked because the `@` picker is about the token the
  // caret sits IN — not the last one in the string. Editing an earlier mention
  // has to reopen the picker there.
  const [caret, setCaret] = useState(0);
  const [highlight, setHighlight] = useState(0);
  // Escape dismisses the picker without dismissing the token: the user keeps
  // typing a slug the roster does not hold, which is their right. Cleared
  // whenever the token itself changes, so the next `@` opens normally.
  const [pickerDismissed, setPickerDismissed] = useState(false);

  const mention =
    mentionables && mentionables.length > 0
      ? activeMention(value, caret)
      : null;
  const matches = mention ? matchMentionables(mentionables!, mention.query) : [];
  const pickerOpen = !!mention && matches.length > 0 && !pickerDismissed;
  const active = matches[Math.min(highlight, matches.length - 1)];

  // Somebody is working. While locked we hide the textarea and show the
  // turn-status line + Pause, rather than leaving the input typeable — rc3 D33:
  // no typing while agents work, and Pause is the only way to take the box back.
  const locked = isLocked(activity, busy);
  // Once the turn actually stops (activity leaves busy/cancelling) drop the
  // local "Cancelling…" spinner. v1 has no explicit backend cancelling state
  // (it goes busy → idle), so this is the post-press feedback.
  useEffect(() => {
    if (!locked) setCancelling(false);
  }, [locked]);

  const handleCancel = async () => {
    if (!onCancel || cancelling) return;
    setCancelling(true);
    try {
      await onCancel();
    } catch (err) {
      setError(errorMessage(err));
      setCancelling(false);
    }
  };

  // The session is paused (Stop landed): textarea stays open for a steer, and
  // the paused bar offers Resume / Close.
  const paused = activity === "paused";
  // Drop the local "Resuming…" latch once the backend leaves paused.
  useEffect(() => {
    if (!paused) setResuming(false);
  }, [paused]);

  const handleResume = async () => {
    if (!onResume || resuming) return;
    setResuming(true);
    try {
      await onResume();
    } catch (err) {
      setError(errorMessage(err));
      setResuming(false);
    }
  };

  /** Replace the token the caret is in with `@slug `, and put the caret after it. */
  const insertMention = (slug: string) => {
    if (!mention) return;
    const next = `${value.slice(0, mention.start)}@${slug} ${value.slice(caret)}`;
    const at = mention.start + slug.length + 2;
    updateValue(next);
    setCaret(at);
    setHighlight(0);
    // The caret move has to wait for React to write the new value, or the
    // browser puts it at the end of the old string.
    requestAnimationFrame(() => {
      const el = textareaRef.current;
      if (!el) return;
      el.focus();
      el.setSelectionRange(at, at);
    });
  };

  const updateValue = (next: string) => {
    setValue(next);
    setPickerDismissed(false);
    if (!draftKey) return;
    // Drop the key entirely when the box is emptied so abandoned sessions
    // don't accumulate "" entries in localStorage.
    if (next) localStorage.setItem(draftKey, next);
    else localStorage.removeItem(draftKey);
  };

  // Auto-grow: reset to `auto` so scrollHeight reflects actual content height,
  // then clamp to 200px (~8 rows). Beyond that the textarea scrolls
  // internally instead of pushing the chat list off-screen.
  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
  }, [value]);

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault();
    const text = value.trim();
    // Staged answers make an empty Send meaningful (rc3 D34): the picks ARE
    // the response, and the backend requires text or at least one pick.
    if ((!text && stagedAnswers === 0) || disabled || sending) return;
    setSending(true);
    setError(null);
    try {
      await onSend(text);
      updateValue("");
    } catch (err) {
      // Keep `value` so the user can retry without retyping, and surface the
      // failure — a silent reject made the user think the message was sent.
      setError(errorMessage(err));
    } finally {
      setSending(false);
    }
  };

  const hint = "↵";

  return (
    <>
      {error && (
        <ErrorBanner
          label="Send failed:"
          message={error}
          onDismiss={() => setError(null)}
          className="mx-3 mt-2"
        />
      )}
      {paused && (
        <div className="flex items-center gap-2 border-b border-outline-variant bg-surface-container-low px-3 py-2">
          <span className="flex-1 text-xs text-on-surface-variant">
            <span className="font-semibold text-on-surface">⏸ Paused</span>
            {" — agents halted. Type below to steer, or"}
          </span>
          {onResume && (
            <Button
              type="button"
              variant="primary"
              onClick={handleResume}
              disabled={resuming}
              className="min-w-[5.5rem]"
              title="Wake the agents and continue where they left off"
            >
              {resuming ? "Resuming…" : "Resume"}
            </Button>
          )}
          {onClose && (
            <Button
              type="button"
              variant="danger"
              onClick={onClose}
              title="Force-close this session (confirmation follows)"
            >
              Close session
            </Button>
          )}
        </div>
      )}
      {/* Unlocked but still working — the locked branch has its own TurnStatus. */}
      {!locked && (
        <StillWorkingNotice
          activity={activity}
          busy={busy}
          label={busyLabel}
          hues={authorHues}
        />
      )}
      <form
        onSubmit={handleSubmit}
        className={cn("flex gap-2 p-3", locked ? "items-center" : "items-end")}
      >
        {locked ? (
          <>
            <TurnStatus
              activity={activity}
              busy={busy}
              label={busyLabel}
              hues={authorHues}
            />
            {onCancel && (
              <Button
                type="button"
                variant="danger"
                onClick={handleCancel}
                // Disabled while the cancel is in flight — either the local press
                // latency (`cancelling`) or the backend's explicit `cancelling`.
                disabled={cancelling || activity === "cancelling"}
                className="min-w-[5.5rem]"
                // Named for what it IS (rc3 D33): the only interrupt in the
                // product. Everything else — a parked question, an approval, a
                // halt — is the session arriving somewhere, not being cut off.
                // It was called "Stop", which reads as an abort; it parks, and
                // Resume picks the ring up where it left off.
                title="Pause the agents — the one interrupt. The session parks until you steer, resume, or close."
              >
                {cancelling || activity === "cancelling"
                  ? "Pausing…"
                  : "Pause"}
              </Button>
            )}
          </>
        ) : (
          <>
            <div className="relative flex-1">
              {pickerOpen && (
                <ul
                  role="listbox"
                  aria-label="Mention a participant"
                  className="absolute bottom-full left-0 z-10 mb-1 max-h-48 w-full overflow-y-auto rounded border border-outline-variant bg-surface-container-lowest py-1 shadow-lg"
                >
                  {matches.map((m, i) => (
                    <li key={m.slug}>
                      <button
                        type="button"
                        role="option"
                        aria-selected={m.slug === active?.slug}
                        // `onMouseDown`, not `onClick`: a click blurs the
                        // textarea first, and the blur closes the picker before
                        // the click can land on it.
                        onMouseDown={(e) => {
                          e.preventDefault();
                          insertMention(m.slug);
                        }}
                        onMouseEnter={() => setHighlight(i)}
                        className={cn(
                          "flex w-full items-baseline gap-2 px-3 py-1.5 text-left text-sm",
                          m.slug === active?.slug
                            ? "bg-surface-container-high text-on-surface"
                            : "text-on-surface-variant",
                        )}
                      >
                        <span
                          className={cn(
                            "font-semibold",
                            authorColorClass(m.label, authorHues),
                          )}
                        >
                          {m.label}
                        </span>
                        <span className="font-mono text-xs opacity-60">
                          @{m.slug}
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
              )}
              <Textarea
                ref={textareaRef}
                rows={2}
                placeholder={placeholder ?? "Message…"}
                value={value}
                onChange={(e) => {
                  updateValue(e.target.value);
                  setCaret(e.target.selectionStart ?? e.target.value.length);
                }}
                // Clicking or arrowing into an earlier `@token` reopens the
                // picker there, so a mention can be fixed rather than retyped.
                onSelect={(e) =>
                  setCaret(e.currentTarget.selectionStart ?? 0)
                }
                onKeyDown={(e) => {
                  // **The picker owns these keys while it is open**, and Enter
                  // most of all: an open picker means the user is mid-mention,
                  // so sending the message on Enter would fire it half-typed.
                  if (pickerOpen) {
                    if (e.key === "ArrowDown") {
                      e.preventDefault();
                      setHighlight((h) => (h + 1) % matches.length);
                      return;
                    }
                    if (e.key === "ArrowUp") {
                      e.preventDefault();
                      setHighlight(
                        (h) => (h - 1 + matches.length) % matches.length,
                      );
                      return;
                    }
                    if (e.key === "Enter" || e.key === "Tab") {
                      e.preventDefault();
                      if (active) insertMention(active.slug);
                      return;
                    }
                    if (e.key === "Escape") {
                      e.preventDefault();
                      setPickerDismissed(true);
                      return;
                    }
                  }
                  // Enter sends; Shift+Enter inserts a newline (so multi-line
                  // messages aren't lost). ⌘/Ctrl+Enter also sends. Skip while an
                  // IME is composing so multibyte input isn't cut mid-character.
                  if (
                    e.key === "Enter" &&
                    !e.shiftKey &&
                    !e.nativeEvent.isComposing
                  ) {
                    e.preventDefault();
                    handleSubmit(e as unknown as FormEvent);
                  }
                }}
                disabled={disabled || sending}
                // Right padding leaves room for the kbd hint overlay.
                className="w-full resize-none pr-14"
              />
              <kbd
                aria-hidden
                className="pointer-events-none absolute bottom-1.5 right-2 select-none rounded border border-outline-variant bg-surface-container-lowest px-1.5 py-0.5 font-mono text-[0.65rem] text-on-surface-variant"
                title="Enter to send · Shift+Enter for a newline"
              >
                {hint}
              </kbd>
            </div>
            {stagedAnswers > 0 && (
              <span
                className="self-center whitespace-nowrap rounded bg-primary/15 px-1.5 py-0.5 text-[0.7rem] text-primary"
                title="Staged tray answers — they travel with this Send as one response"
              >
                +{stagedAnswers} answer{stagedAnswers > 1 ? "s" : ""}
              </span>
            )}
            <Button
              type="submit"
              variant="primary"
              disabled={(!value.trim() && stagedAnswers === 0) || disabled || sending}
              // Fixed min-width so the label cycle (Send → Sending… → Send)
              // doesn't dance the layout on every submit.
              className="min-w-[5.5rem]"
            >
              {sending ? "Sending…" : "Send"}
            </Button>
          </>
        )}
      </form>
    </>
  );
}

// Which participants are mid-turn, as a labelled list — a broadcast can have
// every one of them busy at once. Shared by the locked turn-status line and the
// unlocked still-working notice so the two labels can never drift apart.
//
// One verb for everyone. The old line said Brian "is working" and Rain "is
// reviewing", which is bot-hq claiming to know what a role MEANS; it knows only
// that a participant's turn is in flight (rc3 D10/D11). The colour still comes
// from the slug, matching the same author's chat byline.
function WorkerLine({
  busy,
  label,
  hues,
}: {
  busy?: AgentBusy;
  label?: (slug: string) => string;
  hues?: Record<string, string>;
}) {
  const workers = Object.entries(busy ?? {})
    .filter(([, isBusy]) => isBusy)
    .map(([slug]) => slug);
  return (
    <>
      {workers.map((key, i) => {
        // Resolve ONCE: the label is both what this line prints and what tints
        // it, so a participant keeps one colour here and in its chat byline.
        // Tinting by `key` instead would tint by the busy map's slot key, which
        // no other surface holds — same participant, two colours.
        const shown = label?.(key) ?? UNKNOWN_PARTICIPANT;
        return (
          <span key={key} className="flex items-center gap-1.5">
            {i > 0 && <span className="text-on-surface-variant/40">·</span>}
            <span className={cn("font-semibold", authorColorClass(shown, hues))}>
              {shown}
            </span>
            <span>is working</span>
          </span>
        );
      })}
    </>
  );
}

/**
 * The input is UNLOCKED but a participant is still mid-turn.
 *
 * Under rc3 D33 that combination has exactly ONE cause left: **the user pressed
 * Pause and the busy flags are still draining.** `isLocked` now consults the
 * per-participant map, so a parked question no longer re-opens the box over a
 * running turn — which is what this notice originally existed to apologise for.
 *
 * Keeping the line for the Pause case is the point: the user chose the
 * interrupt, gets the box immediately, and this says the current tool call is
 * still unwinding, so the first reply may land a beat late.
 */
function StillWorkingNotice({
  activity,
  busy,
  label,
  hues,
}: {
  activity?: SessionActivity;
  busy?: AgentBusy;
  label?: (slug: string) => string;
  hues?: Record<string, string>;
}) {
  if (!anyBusy(busy)) return null;
  const paused = activity === "paused";
  return (
    <div className="flex items-center gap-2 border-b border-outline-variant bg-surface-container-low px-3 py-1.5 text-xs text-on-surface-variant">
      <span className="flex flex-wrap items-center gap-x-1.5 gap-y-0.5">
        {activity === "awaiting_user" && <span>Waiting on your answer ·</span>}
        {paused && <span>Stopping ·</span>}
        <WorkerLine busy={busy} label={label} hues={hues} />
        <span>
          {paused
            ? "— finishing the current tool."
            : "— the turn hasn't ended yet."}
        </span>
      </span>
      <BouncingDots />
    </div>
  );
}

// Shown in place of the textarea while a turn is in flight: which participants
// are working, with a little animated spice. The user Stops the turn to reclaim
// the input.
function TurnStatus({
  activity,
  busy,
  label,
  hues,
}: {
  activity?: SessionActivity;
  busy?: AgentBusy;
  label?: (slug: string) => string;
  hues?: Record<string, string>;
}) {
  // A cancel-in-flight reads as "Stopping…" regardless of who was busy.
  if (activity === "cancelling") {
    return (
      <div className="flex flex-1 items-center gap-2 px-1 text-xs text-on-surface-variant">
        <span className="animate-pulse">Stopping the turn…</span>
      </div>
    );
  }
  return (
    <div className="flex flex-1 items-center gap-2 px-1 text-xs text-on-surface-variant">
      <span className="flex flex-wrap items-center gap-x-1.5 gap-y-0.5">
        {anyBusy(busy) ? (
          <WorkerLine busy={busy} label={label} hues={hues} />
        ) : (
          // Locked but no per-agent flag yet (e.g. a stale snapshot): stay generic.
          <span>A participant is working</span>
        )}
      </span>
      <BouncingDots />
    </div>
  );
}

// Three staggered bouncing dots — the "little spice". Decorative; `bg-current`
// inherits the status text colour.
function BouncingDots() {
  return (
    <span className="inline-flex items-end gap-0.5" aria-hidden>
      {[0, 1, 2].map((i) => (
        <span
          key={i}
          className="h-1 w-1 animate-bounce rounded-full bg-current"
          style={{ animationDelay: `${i * 150}ms` }}
        />
      ))}
    </span>
  );
}
