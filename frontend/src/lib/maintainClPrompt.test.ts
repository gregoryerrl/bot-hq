import { describe, it, expect } from "vitest";
import { maintainClPrompt } from "./maintainClPrompt";

describe("maintainClPrompt", () => {
  it("names the project throughout, including the cl_index_search call", () => {
    const p = maintainClPrompt("acme-app");
    expect(p).toContain("acme-app");
    expect(p).toContain('cl_index_search(project="acme-app")');
  });

  it("makes proposal-queue triage part of maintenance", () => {
    const p = maintainClPrompt("acme-app");
    // Triage first — direct writes stale the open queue.
    expect(p).toContain('cl_list_proposals(project="acme-app", status="open")');
    expect(p).toContain("stale");
    // Resolution stays host-only; the session ends with recommendations.
    expect(p).toContain("host-only");
    expect(p).toContain("recommendation");
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
