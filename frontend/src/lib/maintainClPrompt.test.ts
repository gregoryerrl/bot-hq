import { describe, it, expect } from "vitest";
import { maintainClPrompt } from "./maintainClPrompt";

describe("maintainClPrompt", () => {
  it("names the project throughout, including the cl_index_search call", () => {
    const p = maintainClPrompt("acme-app");
    expect(p).toContain("acme-app");
    expect(p).toContain('cl_index_search(project="acme-app")');
  });

  it("applies edits via cl_write_file, with Bash+rescan only for removals", () => {
    const p = maintainClPrompt("acme-app");
    expect(p).toContain('cl_write_file(project="acme-app"');
    expect(p).toContain('cl_rescan(project="acme-app")');
    // The proposal queue is gone — maintenance writes directly, no triage.
    expect(p).not.toContain("cl_propose");
    expect(p).not.toContain("cl_list_proposals");
  });

  it("encodes the study-notes model, all four IPAV phases, and boundaries", () => {
    const p = maintainClPrompt("demo");
    expect(p).toContain("study notes");
    expect(p).toContain("Investigate");
    expect(p).toContain("Plan");
    expect(p).toContain("Apply");
    expect(p).toContain("Verify");
    expect(p).toContain("append-only");
    expect(p).toContain("cl_rescan");
    expect(p).toContain("don't push");
  });
});
