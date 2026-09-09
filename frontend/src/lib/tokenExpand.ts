/**
 * Composer token expansion (round 13, the user's rework): the box keeps the
 * TOKEN — `#bot-hq/conventions.md`, `/n-verify` — the way claude-code does,
 * and the expansion happens once, here, when the text leaves the composer
 * (Send or Stage). Backticks escape: a token inside an inline-code span is
 * the user SHOWING the syntax and passes through literally, matching the
 * backend mention parser's identical heuristic (`core/mentions.rs`).
 */

/** Inside an inline-code span = an odd number of backticks before `index`.
 *  Deliberately simple (a half-typed span escapes the rest of the message —
 *  the safe reading), and shared in spirit with the Rust parser. */
export function insideBacktickSpan(text: string, index: number): boolean {
  let count = 0;
  for (let i = 0; i < index && i < text.length; i++) {
    if (text[i] === "`") count += 1;
  }
  return count % 2 === 1;
}

export type ExpandableItem = { key: string; insert: string };

/** Trailing punctuation a token sheds before matching — `/n-verify.` is the
 *  code plus a full stop, not an unknown code. */
const TRAILING_PUNCT = /[.,;:!?)\]]+$/;

/**
 * Expand every unescaped `#`-document and `/`-promptcode token. `@` mentions
 * are left alone — the backend parses those. Unknown tokens are prose and
 * pass through; replacements are emitted to an output buffer and never
 * re-scanned, so an expansion body containing `/something` cannot cascade.
 *
 * Expansions leave MARKED (round 13, the user: "enclose it in a snippet/box…
 * so the message looks more organized"): a `#` reference lands as inline
 * code (`` `path` ``), a `/` promptcode as a blockquote — chat rows render
 * markdown, so the sent message shows the expansion boxed, and the agents
 * read the same prose either way. The added backticks come in PAIRS, so the
 * odd-count escape heuristic downstream is unaffected.
 */
/** One display segment of composer text: plain prose, or a LIVE token of a
 *  given sigil family. Only tokens that actually RESOLVE get a kind — a dead
 *  token (unknown code, an attachment whose map is gone) segments as plain,
 *  which is exactly the visible difference the chips exist to show. */
export type TokenSegment = {
  text: string;
  kind: "plain" | "mention" | "doc" | "code";
};

/**
 * Split composer text into display segments for the highlight backdrop
 * (round 13, "make them prettier in the input box"). Same walk as
 * [`expandComposerTokens`] — same boundaries, same backtick escape, same
 * trailing-punctuation shedding — so a `#`/`/` chip marks precisely what
 * Send will expand (pinned by the reconstruction test in
 * `tokenExpand.test.ts`, a872dee4 — the parity is a test, not this comment).
 *
 * `@` chips a strict SUBSET of what the backend summons (a872dee4): the Rust
 * parser stops at the first non-`[a-zA-Z0-9-]` and trims trailing `-`, so
 * `@eyes.foo` / `@eyes-` summon chip-less here, and the label-alias path is
 * invisible to this segmenter. Every divergence is the safe direction — a
 * missing chip on a working mention, never a lying chip.
 */
export function tokenSegments(
  text: string,
  mentionSlugs: readonly string[],
  docItems: readonly ExpandableItem[],
  promptcodes: readonly { code: string; prompt: string }[],
): TokenSegment[] {
  const out: TokenSegment[] = [];
  let plain = "";
  const flush = () => {
    if (plain) out.push({ text: plain, kind: "plain" });
    plain = "";
  };
  let i = 0;
  while (i < text.length) {
    const ch = text[i];
    const boundaryOk =
      ch === "/"
        ? i === 0 || /\s/.test(text[i - 1])
        : i === 0 || !/[a-zA-Z0-9]/.test(text[i - 1]);
    const isSigil = ch === "#" || ch === "/" || ch === "@";
    if (isSigil && boundaryOk && !insideBacktickSpan(text, i)) {
      let end = i + 1;
      while (end < text.length && !/\s/.test(text[end])) end += 1;
      const raw = text.slice(i + 1, end);
      const stripped = raw.replace(TRAILING_PUNCT, "");
      const matches = (candidate: string): boolean =>
        ch === "@"
          ? mentionSlugs.some(
              (m) => m.toLowerCase() === candidate.toLowerCase(),
            )
          : ch === "#"
            ? docItems.some((d) => d.key === candidate)
            : promptcodes.some((c) => c.code === candidate);
      const hit = matches(raw) ? raw : matches(stripped) ? stripped : null;
      if (hit !== null) {
        flush();
        out.push({
          text: `${ch}${hit}`,
          kind: ch === "@" ? "mention" : ch === "#" ? "doc" : "code",
        });
        i += 1 + hit.length;
        continue;
      }
    }
    plain += ch;
    i += 1;
  }
  flush();
  return out;
}

export function expandComposerTokens(
  text: string,
  docItems: readonly ExpandableItem[],
  promptcodes: readonly { code: string; prompt: string }[],
): string {
  let out = "";
  let i = 0;
  while (i < text.length) {
    const ch = text[i];
    // `/` opens ONLY at start-of-text or after whitespace (e052ae77): the
    // looser not-alphanumeric rule admitted `.` and `~`, so `run ./test`
    // silently expanded a path segment into the user's `test` promptcode at
    // Send while the box still showed the path. claude-code's own rule.
    // `#` keeps the not-alphanumeric boundary — it has no path-segment shape.
    const boundaryOk =
      ch === "/"
        ? i === 0 || /\s/.test(text[i - 1])
        : i === 0 || !/[a-zA-Z0-9]/.test(text[i - 1]);
    if ((ch === "#" || ch === "/") && boundaryOk && !insideBacktickSpan(text, i)) {
      let end = i + 1;
      while (end < text.length && !/\s/.test(text[end])) end += 1;
      const raw = text.slice(i + 1, end);
      const stripped = raw.replace(TRAILING_PUNCT, "");
      const tail = raw.slice(stripped.length);
      const replacement =
        ch === "#"
          ? docItems.find((d) => d.key === raw)?.insert ??
            (tail ? docItems.find((d) => d.key === stripped)?.insert : undefined)
          : promptcodes.find((c) => c.code === raw)?.prompt ??
            (tail ? promptcodes.find((c) => c.code === stripped)?.prompt : undefined);
      if (replacement !== undefined) {
        const matchedRaw = ch === "#"
          ? docItems.some((d) => d.key === raw)
          : promptcodes.some((c) => c.code === raw);
        // The quote CLOSES with a blank line (review note): without it,
        // CommonMark lazy-continuation pulls the words after the token into
        // the quote — "please /code now" rendered " now" inside the box.
        const marked =
          ch === "#"
            ? `\`${replacement}\``
            : `\n> ${replacement.split("\n").join("\n> ")}\n\n`;
        out += marked + (matchedRaw ? "" : tail);
        i = end;
        continue;
      }
    }
    out += ch;
    i += 1;
  }
  return out;
}
