import { describe, it, expect } from "vitest";
import { docSlugsIn, docTabTarget } from "./docLinks";

describe("docSlugsIn", () => {
  const slugs = ["key-questions", "plan", "decisions-needed", "eyes-handoff"];

  it("links only backticked tokens that are existing slugs, once each, in order", () => {
    expect(
      docSlugsIn(
        "Approve the key questions? They are in `key-questions` (session doc); see also `decisions-needed` and `key-questions` again, and the file `notes.md`.",
        slugs,
      ),
    ).toEqual(["key-questions", "decisions-needed"]);
  });

  it("ignores unbackticked names, commands and unknown slugs", () => {
    expect(docSlugsIn("read key-questions and run `cargo test`; `nope` is not a doc", slugs)).toEqual([]);
    expect(docSlugsIn("", slugs)).toEqual([]);
  });

  it("matches a phase doc's slug too", () => {
    expect(docSlugsIn("The `plan` doc has B0–B5.", slugs)).toEqual(["plan"]);
  });
});

describe("docTabTarget", () => {
  const docs = [
    { slug: "plan", phase: "plan" },
    { slug: "plan-eyes", phase: "plan" },
    { slug: "key-questions", phase: null },
  ];

  it("opens a phase doc and the reviewer's notes in the phase tab, a custom doc in its own", () => {
    expect(docTabTarget("plan", docs)).toEqual({ phase: "plan" });
    expect(docTabTarget("plan-eyes", docs)).toEqual({ phase: "plan" });
    expect(docTabTarget("key-questions", docs)).toEqual({ custom: "key-questions" });
    expect(docTabTarget("nope", docs)).toBeNull();
  });
});
