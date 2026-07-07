import { describe, it, expect } from "vitest";
import { promptEndsWithUnfilledSection } from "./PluginHost";

describe("promptEndsWithUnfilledSection", () => {
  it("flags a prompt whose last non-empty line ends with a colon", () => {
    expect(
      promptEndsWithUnfilledSection(
        "Craft a material.\n\nTask (what material to craft):",
      ),
    ).toBe(true);
  });

  it("ignores trailing blank lines and whitespace after the colon line", () => {
    expect(
      promptEndsWithUnfilledSection("Do the thing.\n\nTask:\n\n   \n"),
    ).toBe(true);
  });

  it("passes a prompt with a filled final section", () => {
    expect(
      promptEndsWithUnfilledSection("Task:\nBuild the oak shelf material."),
    ).toBe(false);
  });

  it("passes plain prose without colons", () => {
    expect(promptEndsWithUnfilledSection("Summarize the CL for me.")).toBe(
      false,
    );
  });

  it("treats empty and whitespace-only prompts as fine (Rust rejects them)", () => {
    expect(promptEndsWithUnfilledSection("")).toBe(false);
    expect(promptEndsWithUnfilledSection("  \n \n")).toBe(false);
  });

  it("only looks at the LAST non-empty line — a colon mid-prompt is fine", () => {
    expect(
      promptEndsWithUnfilledSection("Sections:\n- a\n- b\nDo it now."),
    ).toBe(false);
  });
});
