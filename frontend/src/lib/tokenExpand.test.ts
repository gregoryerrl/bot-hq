import { describe, it, expect } from "vitest";
import {
  expandComposerTokens,
  insideBacktickSpan,
  tokenSegments,
} from "./tokenExpand";
import backtickFixtures from "./backtickFixtures.json";

const DOCS = [
  { key: "doc/investigate", insert: "(session doc: investigate)" },
  { key: "bot-hq/conventions.md", insert: "/Users/x/library/projects/bot-hq/conventions.md" },
];
const CODES = [
  { code: "n-verify", prompt: "Do n rounds of verification." },
];

describe("expandComposerTokens", () => {
  it("expands a promptcode as a BLOCKQUOTE snippet (round 13: marked)", () => {
    expect(expandComposerTokens("please /n-verify now", DOCS, CODES)).toBe(
      "please \n> Do n rounds of verification.\n\n now",
    );
    // Multiline prompts quote every line.
    const codes = [{ code: "two", prompt: "line one\nline two" }];
    expect(expandComposerTokens("/two", DOCS, codes)).toBe(
      "\n> line one\n> line two\n\n",
    );
  });

  it("expands document tokens as inline code", () => {
    expect(expandComposerTokens("read #doc/investigate", DOCS, CODES)).toBe(
      "read `(session doc: investigate)`",
    );
    expect(expandComposerTokens("see #bot-hq/conventions.md", DOCS, CODES)).toBe(
      "see `/Users/x/library/projects/bot-hq/conventions.md`",
    );
  });

  it("sheds trailing punctuation before matching", () => {
    expect(expandComposerTokens("/n-verify.", DOCS, CODES)).toBe(
      "\n> Do n rounds of verification.\n\n.",
    );
  });

  it("a path segment can NEVER trigger a promptcode (e052ae77)", () => {
    // The user's own live code is named `test` — `./test` must stay a path.
    const codes = [{ code: "test", prompt: "EXPANDED" }];
    expect(expandComposerTokens("run ./test now", DOCS, codes)).toBe(
      "run ./test now",
    );
    expect(expandComposerTokens("cd ~/test", DOCS, codes)).toBe("cd ~/test");
    expect(expandComposerTokens("see (/test)", DOCS, codes)).toBe("see (/test)");
    // `/` opens only at start or after whitespace.
    expect(expandComposerTokens("/test", DOCS, codes)).toBe("\n> EXPANDED\n\n");
    expect(expandComposerTokens("say /test now", DOCS, codes)).toBe(
      "say \n> EXPANDED\n\n now",
    );
  });

  it("backticks escape — the user is showing the syntax", () => {
    expect(expandComposerTokens("type `/n-verify` to expand", DOCS, CODES)).toBe(
      "type `/n-verify` to expand",
    );
    expect(
      expandComposerTokens("`#bot-hq/conventions.md` is the token", DOCS, CODES),
    ).toBe("`#bot-hq/conventions.md` is the token");
  });

  it("leaves unknown tokens, paths and mentions alone", () => {
    expect(expandComposerTokens("ls /tmp/foo and #nope", DOCS, CODES)).toBe(
      "ls /tmp/foo and #nope",
    );
    expect(expandComposerTokens("@eyes take a look", DOCS, CODES)).toBe(
      "@eyes take a look",
    );
    // Boundary: a sigil mid-word is prose.
    expect(expandComposerTokens("a/n-verify", DOCS, CODES)).toBe("a/n-verify");
  });

  it("an expansion body is never re-scanned", () => {
    const codes = [{ code: "a", prompt: "use /b here" }, { code: "b", prompt: "BOOM" }];
    expect(expandComposerTokens("/a", DOCS, codes)).toBe("\n> use /b here\n\n");
  });
});

describe("tokenSegments", () => {
  const SLUGS = ["eyes", "hands"];

  it("marks live tokens by family and leaves prose plain", () => {
    expect(
      tokenSegments("ask @eyes about #doc/investigate then /n-verify", SLUGS, DOCS, CODES),
    ).toEqual([
      { text: "ask ", kind: "plain" },
      { text: "@eyes", kind: "mention" },
      { text: " about ", kind: "plain" },
      { text: "#doc/investigate", kind: "doc" },
      { text: " then ", kind: "plain" },
      { text: "/n-verify", kind: "code" },
    ]);
  });

  it("a DEAD token segments as plain — the visible difference", () => {
    expect(tokenSegments("#gone.png and /nope", SLUGS, DOCS, CODES)).toEqual([
      { text: "#gone.png and /nope", kind: "plain" },
    ]);
  });

  it("backticks and boundaries match the expander", () => {
    expect(tokenSegments("`/n-verify` cd ./n-verify", SLUGS, DOCS, CODES)).toEqual([
      { text: "`/n-verify` cd ./n-verify", kind: "plain" },
    ]);
    // Trailing punctuation stays OUTSIDE the chip.
    expect(tokenSegments("(/n-verify.)", SLUGS, DOCS, CODES)).toEqual([
      { text: "(/n-verify.)", kind: "plain" },
    ]);
    expect(tokenSegments("/n-verify.", SLUGS, DOCS, CODES)).toEqual([
      { text: "/n-verify", kind: "code" },
      { text: ".", kind: "plain" },
    ]);
  });
});

describe("segments ↔ expansion parity (a872dee4)", () => {
  // The round's third hand-mirror, pinned like the other two: rebuild the
  // Send-time expansion FROM the display segments and require byte-equality
  // with expandComposerTokens over a table that exercises every family,
  // escapes, boundaries, punctuation tails and dead tokens. A walk change in
  // either function that the other doesn't mirror reddens here.
  const SLUGS = ["eyes", "hands"];
  const TABLE = [
    "ask @eyes about #doc/investigate then /n-verify",
    "dead: #gone.png and /nope stay",
    "`/n-verify` cd ./n-verify ~/n-verify",
    "(/n-verify.) and /n-verify. at large",
    "#bot-hq/conventions.md. tail",
    "plain prose only",
    "` unclosed /n-verify #doc/investigate",
    "@eyes.foo @eyes- @nobody",
  ];

  function rebuild(text: string): string {
    return tokenSegments(text, SLUGS, DOCS, CODES)
      .map((seg) => {
        if (seg.kind === "plain" || seg.kind === "mention") return seg.text;
        if (seg.kind === "doc") {
          const item = DOCS.find((d) => `#${d.key}` === seg.text)!;
          return `\`${item.insert}\``;
        }
        const code = CODES.find((c) => `/${c.code}` === seg.text)!;
        return `\n> ${code.prompt.split("\n").join("\n> ")}\n\n`;
      })
      .join("");
  }

  it("the expansion is exactly the segments, re-expanded", () => {
    for (const text of TABLE) {
      expect(rebuild(text), text).toBe(expandComposerTokens(text, DOCS, CODES));
    }
  });
});

describe("insideBacktickSpan", () => {
  it("odd count before the index means inside", () => {
    const t = "a `b` c";
    expect(insideBacktickSpan(t, 3)).toBe(true);
    expect(insideBacktickSpan(t, 6)).toBe(false);
  });

  it("agrees with the Rust mention parser on the SHARED fixture table", () => {
    // `backtickFixtures.json` is read by BOTH this test and
    // `core/mentions.rs::the_two_backtick_heuristics_share_one_fixture_table`
    // — the two implementations of the odd-backtick rule are hand-mirrored
    // (TS guards the pickers/expander, Rust guards `@` parsing), and this
    // table is what reddens if they diverge (review note on 2f7a511).
    for (const f of backtickFixtures) {
      expect(insideBacktickSpan(f.text, f.index), `${f.text} @ ${f.index}`).toBe(
        f.inside,
      );
    }
  });
});
