import { describe, it, expect } from "vitest";
import { docSlugsIn } from "./docLinks";

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
