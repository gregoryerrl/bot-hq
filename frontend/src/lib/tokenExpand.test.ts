import { describe, it, expect } from "vitest";
import { expandComposerTokens, insideBacktickSpan } from "./tokenExpand";
import backtickFixtures from "./backtickFixtures.json";

const DOCS = [
  { key: "doc/investigate", insert: "(session doc: investigate)" },
  { key: "bot-hq/conventions.md", insert: "/Users/x/library/projects/bot-hq/conventions.md" },
];
const CODES = [
  { code: "n-verify", prompt: "Do n rounds of verification." },
];

describe("expandComposerTokens", () => {
  it("expands a promptcode token at send, keeping surrounding prose", () => {
    expect(expandComposerTokens("please /n-verify now", DOCS, CODES)).toBe(
      "please Do n rounds of verification. now",
    );
  });

  it("expands document tokens to their references", () => {
    expect(expandComposerTokens("read #doc/investigate", DOCS, CODES)).toBe(
      "read (session doc: investigate)",
    );
    expect(expandComposerTokens("see #bot-hq/conventions.md", DOCS, CODES)).toBe(
      "see /Users/x/library/projects/bot-hq/conventions.md",
    );
  });

  it("sheds trailing punctuation before matching", () => {
    expect(expandComposerTokens("/n-verify.", DOCS, CODES)).toBe(
      "Do n rounds of verification..",
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
    expect(expandComposerTokens("/test", DOCS, codes)).toBe("EXPANDED");
    expect(expandComposerTokens("say /test now", DOCS, codes)).toBe(
      "say EXPANDED now",
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
    expect(expandComposerTokens("/a", DOCS, codes)).toBe("use /b here");
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
