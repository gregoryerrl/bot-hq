import { useEffect, useRef, useState } from "react";
import { Button } from "./ui/Button";
import { Textarea } from "./ui/Textarea";
import { ErrorBanner } from "./ErrorBanner";
import { errorMessage } from "../hooks/useInvoke";
import { cn } from "../lib/cn";
import { authorColorClass } from "./authorColor";
import { UNKNOWN_PARTICIPANT } from "../lib/participants";
import { anyBusy, isLocked, type AgentBusy, type SessionActivity } from "../stores/activity";
import { uriListToPaths } from "../lib/filePaste";
import { expandComposerTokens, insideBacktickSpan } from "../lib/tokenExpand";

/** One participant the `@` picker can insert (rc3 D17). */
type Mentionable = {
  /** What goes in the text, after the `@`. The backend parses this. */
  slug: string;
  /** What the user reads — the display rule's `ROLE · Model`. */
  label: string;
};

/** One row any sigil picker can offer: what to read, what to insert. */
type PickerItem = {
  /** The short handle after the sigil (`@slug` / `#doc-slug` / `/code`) —
   *  matched by prefix, printed as the mono hint. */
  key: string;
  /** What the user reads — role · model, a doc title, a code's first line. */
  label: string;
  /** The text the pick puts in the box (a trailing space is added for it). */
  insert: string;
};

/**
 * The sigil-token the caret is sitting in, or `null`.
 *
 * Generalized from the `@` walker (rc3 D17) for `#` documents and `/`
 * promptcodes — same walk, same boundaries, so the three pickers cannot
 * drift: walks BACK from the caret to the nearest sigil in `sigils`, giving
 * up at whitespace — `@adv|` is a live token and `@adv thoughts|` is not,
 * which is what makes a picker close by itself once the user moves on.
 *
 * The boundary rule matches the backend parser (`core::mentions`) rather than
 * approximating it: a sigil preceded by an alphanumeric is part of a word —
 * an email address for `@`, a path segment for `/`, `issue#5` for `#` — and
 * offering a picker there would suggest bot-hq is about to do something it
 * will not.
 */
function activeToken(
  text: string,
  caret: number,
  sigils: string,
): { sigil: string; start: number; query: string } | null {
  let i = caret;
  while (i > 0) {
    const ch = text[i - 1];
    if (sigils.includes(ch)) {
      const before = i >= 2 ? text[i - 2] : undefined;
      // `/` opens only at start-of-text or after whitespace (e052ae77): the
      // not-alphanumeric rule admitted `.`/`~`/`(`, so typing a path like
      // `./test` opened the picker over a segment that must stay a path.
      // Mirrors `expandComposerTokens` exactly — picker and expander must
      // agree on what is a token.
      if (ch === "/") {
        if (before !== undefined && !/\s/.test(before)) return null;
      } else if (before !== undefined && /[a-zA-Z0-9]/.test(before)) {
        return null;
      }
      return { sigil: ch, start: i - 1, query: text.slice(i, caret) };
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
function matchPickerItems(all: PickerItem[], query: string): PickerItem[] {
  const q = query.toLowerCase();
  if (!q) return all;
  return all.filter((m) => {
    if (m.key.toLowerCase().startsWith(q)) return true;
    return m.label
      .toLowerCase()
      .split(/[^a-z0-9]+/)
      .some((word) => word.startsWith(q));
  });
}

interface ChatInputProps {
  placeholder?: string;
  onSend: (text: string) => Promise<void> | void;
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
   * Internal documents the `#` picker can reference — this session's IPAV /
   * custom docs and the Context Library's files (ideas.md, 2026-08-24). Picking
   * one inserts its `insert` text: an absolute path for a CL file (agents Read
   * it), a `(session doc: slug)` reference for a session doc (agents hold
   * `session_doc_read`). Insertion happens at pick time — what you see in the
   * box is exactly what sends; nothing expands later. Absent or empty, `#`
   * opens nothing.
   */
  docMentionables?: PickerItem[];
  /**
   * User-configured `/` promptcodes (Settings → Promptcodes; ideas.md,
   * 2026-08-24): short codes for prompts typed often. Picking one REPLACES the
   * token with the full prompt text — the box shows exactly what will send,
   * nothing expands later. Absent or empty, `/` opens nothing (so `foo/bar`
   * and `./bin` never summon a picker: a sigil preceded by an alphanumeric is
   * part of a word, and a query matching no code renders no listbox).
   */
  promptcodes?: { code: string; prompt: string }[];
  /**
   * Save clipboard IMAGE bytes and return the absolute path to insert
   * (ideas.md 2026-08-24 — paste files into the box). Supplied by the parent
   * because saving needs the session id; without it an image paste is ignored.
   * File paths (Finder copy, drag-drop) need no saving — they insert directly.
   */
  savePastedImage?: (bytes: Uint8Array, ext: string) => Promise<string>;
  /**
   * The session's activity. While `busy`/`cancelling` the textarea stays
   * writable but the submit slot becomes **Stage** (queued for the next turn
   * boundary) beside a turn-status line (which participants are working) and
   * Pause — the one interrupt. `idle` / `awaiting_user` show the textarea +
   * Send.
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
  /**
   * The Stage toggle (2026-08-15). While the ring runs the box stays
   * WRITABLE — composing was never the hazard; landing mid-turn was — and
   * the Send slot becomes **Stage**: toggled on, the message locks and the
   * backend delivers it at the next turn boundary together with the staged
   * tray answers, exactly like a typed Send. Untoggling makes it editable
   * again. Pause remains the only interrupt.
   */
  staged?: boolean;
  /** The staged text, for rehydrating the box after a reload. */
  stagedText?: string | null;
  /** Increments each time a staged response DELIVERS — clears the draft. */
  deliveredTick?: number;
  onStage?: (text: string) => Promise<void> | void;
  onUnstage?: () => Promise<void> | void;
}

/**
 * The localStorage key a session's composer draft persists under.
 *
 * One function because two production sites need the SAME string and nothing
 * kept them equal: `SessionView` both passes it as `draftKey` and removes it
 * when a staged delivery lands. Changing the format at one site would leave the
 * other reading a key nobody writes, and the delivered-draft clear would stop
 * silently — the failure has no symptom until a user notices their message came
 * back. One restatement too many for a fact that must never disagree with
 * itself.
 */
export function draftKeyFor(sessionId: string): string {
  return `bothq:draft:${sessionId}`;
}

/** Suffix deriving the attachment-map key from a draft key — ONE literal, so
 *  ChatInput (which only has the generic `draftKey` prop) and Providers
 *  (which clears by session id) cannot drift apart. */
export const DRAFT_FILES_SUFFIX = ":files";

/** Where a session's attachment token→path map persists (cce52574): the
 *  sibling of its composer draft, cleared everywhere the draft is. */
export function draftFilesKeyFor(sessionId: string): string {
  return `${draftKeyFor(sessionId)}${DRAFT_FILES_SUFFIX}`;
}

/** Parse a persisted attachment map; garbage reads as "no attachments". */
function readAttachedFiles(key: string | undefined): PickerItem[] {
  if (!key) return [];
  try {
    const parsed = JSON.parse(localStorage.getItem(key) ?? "[]") as unknown;
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (f): f is PickerItem =>
        typeof f === "object" &&
        f !== null &&
        typeof (f as { key?: unknown }).key === "string" &&
        typeof (f as { insert?: unknown }).insert === "string",
    );
  } catch {
    return [];
  }
}

export function ChatInput({
  placeholder,
  onSend,
  mentionables,
  docMentionables,
  promptcodes,
  savePastedImage,
  activity,
  busy,
  stagedAnswers = 0,
  busyLabel,
  authorHues,
  onCancel,
  onResume,
  onClose,
  draftKey,
  staged = false,
  stagedText = null,
  deliveredTick = 0,
  onStage,
  onUnstage,
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
  // Files the user dropped/pasted into this composer (round 13): each gets a
  // short `#name.ext` token in the box and this map carries it back to the
  // real path at Send/Stage. **Persisted beside the draft** (cce52574): the
  // draft text survives a remount — switching sessions is the user's normal
  // workflow — so a memory-only map left the restored `#token` pixel-identical
  // to a live one while silently sending literal prose. Restored lazily like
  // the draft itself; cleared wherever the draft is.
  const filesKey = draftKey ? `${draftKey}${DRAFT_FILES_SUFFIX}` : undefined;
  const [attachedFiles, setAttachedFiles] = useState<PickerItem[]>(() =>
    readAttachedFiles(filesKey),
  );

  // A sigil is live only while it has something to offer — with no roster,
  // `@` opens nothing, exactly as before; same rule for `#`.
  const sigils =
    (mentionables && mentionables.length > 0 ? "@" : "") +
    (docMentionables && docMentionables.length > 0 ? "#" : "") +
    (promptcodes && promptcodes.length > 0 ? "/" : "");
  const rawToken = sigils ? activeToken(value, caret, sigils) : null;
  // Backticks escape every sigil (round 13): typing `` `/n-verify` `` is the
  // user SHOWING the token, and a picker over it would say otherwise.
  const token =
    rawToken && !insideBacktickSpan(value, rawToken.start) ? rawToken : null;
  const tokenItems: PickerItem[] =
    token === null
      ? []
      : token.sigil === "@"
        ? (mentionables ?? []).map((m) => ({
            key: m.slug,
            label: m.label,
            insert: `@${m.slug}`,
          }))
        : token.sigil === "#"
          ? [...(docMentionables ?? []), ...attachedFiles]
          : (promptcodes ?? []).map((c) => {
              const firstLine = c.prompt.split("\n", 1)[0] ?? "";
              return {
                key: c.code,
                label:
                  firstLine.length > 60
                    ? `${firstLine.slice(0, 59)}…`
                    : firstLine,
                insert: c.prompt,
              };
            });
  const matches = token ? matchPickerItems(tokenItems, token.query) : [];
  const pickerOpen = !!token && matches.length > 0 && !pickerDismissed;
  const active = matches[Math.min(highlight, matches.length - 1)];

  // Somebody is working. The box stays WRITABLE (the Stage toggle,
  // 2026-08-15) — rc3 D33's rule was always about messages LANDING mid-turn,
  // not about composing — but nothing SENDS while locked: the submit slot
  // becomes Stage, delivery waits for a turn boundary, and Pause stays the
  // only interrupt.
  const locked = isLocked(activity, busy);
  // Once the turn actually stops (activity leaves busy/cancelling) drop the
  // local "Cancelling…" spinner. v1 has no explicit backend cancelling state
  // (it goes busy → idle), so this is the post-press feedback.
  useEffect(() => {
    if (!locked) setCancelling(false);
  }, [locked]);
  const [staging, setStaging] = useState(false);
  // A staged delivery consumed the message: clear the draft. Ref-guarded so
  // the mount value never wipes a draft.
  const prevTickRef = useRef(deliveredTick);
  useEffect(() => {
    if (prevTickRef.current !== deliveredTick) {
      prevTickRef.current = deliveredTick;
      updateValue("");
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [deliveredTick]);
  // Rehydrate the box from the backend's staged content (a reload while
  // staged). Only while staged — the box is readOnly then, so this can never
  // fight the user's typing.
  //
  // This effect is CORRECT and is not what refilled the box after a delivery:
  // its deps are `[staged, stagedText]`, and during the post-delivery window
  // neither changes, so it does not run. (Checked by kill test — guarding it
  // against a "just delivered" flag changed nothing.) The box refilled because
  // the delivered message was genuinely re-staged behind it; see the re-stage
  // effect in `SessionView`.
  useEffect(() => {
    if (staged && stagedText != null && stagedText !== value) {
      setValue(stagedText);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [staged, stagedText]);

  const handleStage = async () => {
    if (!onStage || staging || staged) return;
    const text = value.trim();
    if (!text && stagedAnswers === 0) return;
    setStaging(true);
    setError(null);
    try {
      // The box keeps the tokens; what leaves the composer is expanded. The
      // staged snapshot is therefore the EXPANDED text (a reload-while-staged
      // rehydrates it — the deliverable, not the shorthand).
      await onStage(
        expandComposerTokens(
          text,
          [...(docMentionables ?? []), ...attachedFiles],
          promptcodes ?? [],
        ),
      );
      // The text stays in the (now read-only) box — it IS the staged message.
      // But the DRAFT key is dropped: from here the backend holds the text
      // (`stagedText` rehydrates the box after a reload), and the key was what
      // put a delivered message back in the box when the delivery happened
      // with this session's view unmounted — on the dashboard, or in another
      // session — where no `session:stage_delivered` handler could clear it
      // (issues.md #3, the "sometimes"). Nothing to persist means nothing to
      // resurrect; `handleUnstage` writes the draft back when editing resumes.
      if (draftKey) localStorage.removeItem(draftKey);
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setStaging(false);
    }
  };

  const handleUnstage = async () => {
    if (!onUnstage) return;
    setError(null);
    try {
      await onUnstage();
      // Editing resumes: the text is a draft again, so it persists again.
      if (draftKey && value) localStorage.setItem(draftKey, value);
    } catch (err) {
      setError(errorMessage(err));
    }
  };

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

  /** Replace the token the caret is in with its CANONICAL TOKEN + a space —
   *  `@slug` / `#key` / `/code` — never the expansion (round 13, the user:
   *  "I want to see the actual `#filehere` and `/promptcodehere`"). The
   *  expansion happens once, at Send/Stage, in `expandComposerTokens`. */
  const insertItem = (item: PickerItem) => {
    if (!token) return;
    const inserted = `${token.sigil}${item.key} `;
    const next = `${value.slice(0, token.start)}${inserted}${value.slice(caret)}`;
    const at = token.start + inserted.length;
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

  /** Insert text at the caret (drop/paste of file paths), keeping the box
   *  editable state rules: a staged (read-only) box refuses the insert. */
  const insertAtCaret = (text: string) => {
    if (!text || staged) return;
    const el = textareaRef.current;
    const at = el?.selectionStart ?? value.length;
    const next = `${value.slice(0, at)}${text}${value.slice(at)}`;
    updateValue(next);
    const after = at + text.length;
    setCaret(after);
    requestAnimationFrame(() => {
      const box = textareaRef.current;
      if (!box) return;
      box.focus();
      box.setSelectionRange(after, after);
    });
  };
  const insertAtCaretRef = useRef(insertAtCaret);
  insertAtCaretRef.current = insertAtCaret;

  /** Register dropped/pasted paths and insert their short `#` tokens at the
   *  caret — the sigil look the user asked for, instead of raw paths. Keys
   *  are sanitized basenames, deduped with `-2`-style suffixes against both
   *  earlier attachments and the document list. */
  const attachPaths = (paths: string[]) => {
    if (paths.length === 0) return;
    // Computed OUTSIDE the state updater (review note on 9a43c5b): the
    // updater must stay pure — the first cut ran the caret insert inside it,
    // which was safe only because the insert read `value` non-functionally.
    // Drops are user-paced, so the closure read of `attachedFiles` cannot
    // race a second attach in the same tick.
    const prev = attachedFiles;
    const taken = new Set<string>([
      ...prev.map((f) => f.key),
      ...(docMentionables ?? []).map((d) => d.key),
    ]);
    const added: PickerItem[] = [];
    for (const path of paths) {
      const existing = prev.find((f) => f.insert === path);
      if (existing) {
        added.push(existing);
        continue;
      }
      const base = (path.split("/").pop() || "file").replace(/\s+/g, "-");
      let key = base;
      for (let n = 2; taken.has(key); n += 1) {
        const dot = base.lastIndexOf(".");
        key = dot > 0 ? `${base.slice(0, dot)}-${n}${base.slice(dot)}` : `${base}-${n}`;
      }
      taken.add(key);
      added.push({ key, label: key, insert: path });
    }
    const fresh = added.filter((f) => !prev.includes(f));
    if (fresh.length > 0) {
      const next = [...prev, ...fresh];
      setAttachedFiles(next);
      if (filesKey) localStorage.setItem(filesKey, JSON.stringify(next));
    }
    insertAtCaretRef.current(`${added.map((f) => `#${f.key}`).join(" ")} `);
  };

  const attachPathsRef = useRef(attachPaths);
  attachPathsRef.current = attachPaths;

  // Files dragged onto the window insert as short tokens. Tauri v2 keeps
  // dragDropEnabled on (HTML5 drop never fires) and its event carries real OS
  // paths — the thing a DOM drop cannot give. Registered while the composer is
  // mounted; the gate and closed-session bars unmount it, so a drop only ever
  // lands here when the box is on screen. Dynamic import + catch: outside a
  // Tauri webview (vitest/jsdom) registration quietly does nothing.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    void (async () => {
      try {
        const { getCurrentWebview } = await import("@tauri-apps/api/webview");
        const un = await getCurrentWebview().onDragDropEvent((event) => {
          if (event.payload.type !== "drop") return;
          const paths = event.payload.paths ?? [];
          if (paths.length > 0) attachPathsRef.current(paths);
        });
        if (cancelled) un();
        else unlisten = un;
      } catch {
        // Not running inside a Tauri webview — nothing to register.
      }
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  /** Paste: a Finder-copied file arrives as text/uri-list → insert its path;
   *  clipboard IMAGE data is saved via `savePastedImage` → insert the saved
   *  path. Plain text pastes fall through to the browser default. */
  const handlePaste = (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
    const data = e.clipboardData;
    if (!data) return;
    const uriList = data.getData("text/uri-list");
    const paths = uriList ? uriListToPaths(uriList) : [];
    if (paths.length > 0) {
      e.preventDefault();
      attachPaths(paths);
      return;
    }
    if (!savePastedImage) return;
    const image = Array.from(data.items).find(
      (item) => item.kind === "file" && item.type.startsWith("image/"),
    );
    const file = image?.getAsFile();
    if (!file) return;
    e.preventDefault();
    void (async () => {
      try {
        const ext = (file.type.split("/")[1] ?? "png").replace("jpeg", "jpg");
        const bytes = new Uint8Array(await file.arrayBuffer());
        const path = await savePastedImage(bytes, ext);
        attachPathsRef.current([path]);
      } catch (err) {
        setError(errorMessage(err));
      }
    })();
  };

  const updateValue = (next: string) => {
    setValue(next);
    setPickerDismissed(false);
    if (!draftKey) return;
    // Drop the key entirely when the box is emptied so abandoned sessions
    // don't accumulate "" entries in localStorage.
    if (next) localStorage.setItem(draftKey, next);
    else {
      localStorage.removeItem(draftKey);
      // The attachment map travels with the draft (cce52574): an emptied box
      // has no tokens left for it to resolve.
      if (filesKey) localStorage.removeItem(filesKey);
      setAttachedFiles([]);
    }
  };

  // Auto-grow: reset to `auto` so scrollHeight reflects actual content height,
  // then clamp to 200px (~8 rows). Beyond that the textarea scrolls
  // internally instead of pushing the chat list off-screen. `scrollHeight` is
  // the padding box and the element is `border-box` with a 1px border, so the
  // two border widths are added back — without them the box sat 2px short of
  // its own content and always had a hair of internal scroll (round 11).
  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight + 2, 200)}px`;
  }, [value]);

  // Typed to what it uses (`preventDefault`) rather than `FormEvent`, so the
  // keyboard path can call it with its own event and no cast.
  const handleSubmit = async (e: { preventDefault(): void }) => {
    e.preventDefault();
    // While the ring runs, submit means STAGE — nothing ever sends mid-turn.
    if (locked) {
      await handleStage();
      return;
    }
    const text = value.trim();
    // Staged answers make an empty Send meaningful (rc3 D34): the picks ARE
    // the response, and the backend requires text or at least one pick.
    if ((!text && stagedAnswers === 0) || sending) return;
    setSending(true);
    setError(null);
    try {
      await onSend(
        expandComposerTokens(
          text,
          [...(docMentionables ?? []), ...attachedFiles],
          promptcodes ?? [],
        ),
      );
      updateValue("");
    } catch (err) {
      // Keep `value` so the user can retry without retyping, and surface the
      // failure — a silent reject made the user think the message was sent.
      setError(errorMessage(err));
    } finally {
      setSending(false);
    }
  };

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
      {/* Two rows, always: the box, then ONE footer row — status on the left
          (which participants are working / the Pause drain), the staged-answer
          chip beside it, Stage|Send and Pause on the right. Round 11: the
          buttons used to sit BESIDE the box, bottom-aligned, so an auto-grown
          textarea (2 → 8 rows) left a blank column above them that grew as the
          user typed, and the locked state spent a third row on the status
          line alone. */}
      <form onSubmit={handleSubmit} className="flex flex-col gap-1.5 p-3">
        {/* `min-w-0`: a flex/grid child defaults to `min-width:auto` and a
            textarea's min-content is ~20 characters, so at the narrow end of
            the split the row overflowed the pane (round 11). */}
        <div className="relative min-w-0">
          {pickerOpen && (
            <ul
              role="listbox"
              aria-label={
                token?.sigil === "#"
                  ? "Mention a document"
                  : token?.sigil === "/"
                    ? "Insert a promptcode"
                    : "Mention a participant"
              }
              className="absolute bottom-full left-0 z-10 mb-1 max-h-48 w-full overflow-y-auto overflow-x-hidden rounded border border-outline-variant bg-surface-container-lowest py-1 shadow-lg"
            >
              {matches.map((m, i) => (
                <li key={m.key}>
                  <button
                    type="button"
                    role="option"
                    aria-selected={m.key === active?.key}
                    // `onMouseDown`, not `onClick`: a click blurs the
                    // textarea first, and the blur closes the picker before
                    // the click can land on it.
                    onMouseDown={(e) => {
                      e.preventDefault();
                      insertItem(m);
                    }}
                    onMouseEnter={() => setHighlight(i)}
                    // `min-w-0` + the per-span truncate: the label is
                    // USER-TYPED (rc3 D20), so this row's width is user
                    // controlled. A flex child defaults to `min-width:auto`
                    // and refuses to shrink below its content, so without
                    // this a long label widens the row past the picker and
                    // the container scrolls sideways — which the pair above
                    // now clips into invisibility rather than fixing.
                    className={cn(
                      "flex w-full min-w-0 items-baseline gap-2 px-3 py-1.5 text-left text-sm",
                      m.key === active?.key
                        ? "bg-surface-container-high text-on-surface"
                        : "text-on-surface-variant",
                    )}
                  >
                    <span
                      className={cn(
                        "truncate font-semibold",
                        authorColorClass(m.label, authorHues),
                      )}
                    >
                      {m.label}
                    </span>
                    <span className="shrink-0 font-mono text-xs opacity-60">
                      {token?.sigil}
                      {m.key}
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
            onPaste={handlePaste}
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
                  if (active) insertItem(active);
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
                handleSubmit(e);
              }
            }}
            disabled={sending || activity === "cancelling"}
            readOnly={staged}
            title="Enter to send · Shift+Enter for a newline"
            className={cn("w-full resize-none", staged && "opacity-80")}
          />
        </div>
        <div className="flex items-center gap-2">
          <div className="flex min-w-0 flex-1 flex-wrap items-center gap-x-2 gap-y-0.5 text-xs text-on-surface-variant">
            {locked ? (
              <TurnStatus
                activity={activity}
                busy={busy}
                label={busyLabel}
                hues={authorHues}
              />
            ) : (
              <StillWorkingNotice busy={busy} label={busyLabel} hues={authorHues} />
            )}
            {stagedAnswers > 0 && (
              <span
                className="whitespace-nowrap rounded bg-primary/15 px-1.5 py-0.5 text-[0.7rem] text-primary"
                title="Staged tray answers — they travel with this response as one Send"
              >
                +{stagedAnswers} answer{stagedAnswers > 1 ? "s" : ""}
              </span>
            )}
          </div>
          {locked ? (
            staged ? (
              <Button
                type="button"
                variant="secondary"
                onClick={handleUnstage}
                className="min-w-[5.5rem]"
                title="Staged — delivers at the next turn break, with your staged answers. Click to edit."
              >
                Staged ✓
              </Button>
            ) : (
              <Button
                type="submit"
                variant="primary"
                disabled={
                  (!value.trim() && stagedAnswers === 0) ||
                  !onStage ||
                  staging ||
                  activity === "cancelling"
                }
                className="min-w-[5.5rem]"
                title="Queue this message — it delivers at the next turn break, never mid-turn. Pause is the interrupt."
              >
                {staging ? "Staging…" : "Stage"}
              </Button>
            )
          ) : (
            <Button
              type="submit"
              variant="primary"
              disabled={(!value.trim() && stagedAnswers === 0) || sending}
              // Fixed min-width so the label cycle (Send → Sending… → Send)
              // doesn't dance the layout on every submit.
              className="min-w-[5.5rem]"
              title="Send — Enter · Shift+Enter for a newline"
            >
              {sending ? "Sending…" : "Send"}
            </Button>
          )}
          {locked && onCancel && (
            <Button
              type="button"
              variant="danger"
              onClick={handleCancel}
              // Disabled while the cancel is in flight — either the local
              // press latency (`cancelling`) or the backend's explicit state.
              disabled={cancelling || activity === "cancelling"}
              className="min-w-[5.5rem]"
              // Named for what it IS (rc3 D33): the only interrupt in the
              // product. Everything else — a parked question, an approval, a
              // halt — is the session arriving somewhere, not being cut off.
              title="Pause the agents — the one interrupt. The session parks until you steer, resume, or close."
            >
              {cancelling || activity === "cancelling" ? "Pausing…" : "Pause"}
            </Button>
          )}
        </div>
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
  busy,
  label,
  hues,
}: {
  busy?: AgentBusy;
  label?: (slug: string) => string;
  hues?: Record<string, string>;
}) {
  // Rendered only when the box is UNLOCKED and someone is busy — which
  // `isLocked` allows for exactly one activity, `paused` (round 10: the
  // former `awaiting_user` line and the "turn hasn't ended" arm were
  // unreachable, and are gone).
  if (!anyBusy(busy)) return null;
  return (
    <span className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5">
      <span>Stopping ·</span>
      <WorkerLine busy={busy} label={label} hues={hues} />
      <span>— finishing the current tool.</span>
      <BouncingDots />
    </span>
  );
}

// Rendered in the composer's footer row while a turn is in flight (the
// textarea stays mounted and writable under the lock since the Stage toggle,
// 2026-08-15): which participants are working, with a little animated spice.
// The user Pauses to interrupt.
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
    return <span className="animate-pulse">Stopping the turn…</span>;
  }
  return (
    <span className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5">
      {anyBusy(busy) ? (
        <WorkerLine busy={busy} label={label} hues={hues} />
      ) : (
        // Locked but no per-agent flag yet (e.g. a stale snapshot): stay generic.
        <span>A participant is working</span>
      )}
      <BouncingDots />
    </span>
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
