import { describe, it, expect } from "vitest";
import { gateChipsOf, looksLikeGateId } from "./gateChips";

describe("gateChipsOf", () => {
  const row = (choice_id: string, status: string, picked: string | null = null, command: string | null = "gh pr merge 5") => ({
    choice_id,
    status,
    command_text: command,
    picked_option: picked,
  });

  it("maps the session's gates by 8-hex prefix with their state", () => {
    const chips = gateChipsOf([
      row("236473a7-0000-4000-8000-000000000001", "answered", "Approve"),
      row("fbc71282-0000-4000-8000-000000000002", "answered", "not yet, CI is red"),
      row("c2d52757-0000-4000-8000-000000000003", "pending"),
      row("aaaaaaaa-0000-4000-8000-000000000004", "withdrawn"),
      // A question, not a gate: no chip.
      row("bbbbbbbb-0000-4000-8000-000000000005", "pending", null, null),
    ]);
    expect(chips.get("236473a7")?.state).toBe("approved");
    expect(chips.get("fbc71282")?.state).toBe("rejected");
    expect(chips.get("c2d52757")?.state).toBe("pending");
    expect(chips.get("aaaaaaaa")?.state).toBe("withdrawn");
    expect(chips.has("bbbbbbbb")).toBe(false);
    expect(chips.get("236473a7")?.choiceId).toBe("236473a7-0000-4000-8000-000000000001");
  });

  it("reads an approved gate's RUN outcome, not just the approval (feedback #104)", () => {
    const approved = (id: string, extra: { exit_code?: number | null; run_refusal?: string | null }) => ({
      ...row(id, "answered", "Approve"),
      ...extra,
    });
    const chips = gateChipsOf([
      approved("aaaaaaaa-0000-4000-8000-000000000001", {}),
      approved("bbbbbbbb-0000-4000-8000-000000000002", { exit_code: 0 }),
      approved("cccccccc-0000-4000-8000-000000000003", { exit_code: 1 }),
      approved("dddddddd-0000-4000-8000-000000000004", { exit_code: 124 }),
      approved("eeeeeeee-0000-4000-8000-000000000005", { run_refusal: "the run window passed" }),
    ]);
    expect(chips.get("aaaaaaaa")).toMatchObject({ state: "approved", label: "approved" });
    expect(chips.get("bbbbbbbb")).toMatchObject({ state: "ran", label: "ran" });
    expect(chips.get("cccccccc")).toMatchObject({ state: "failed", label: "exit 1" });
    expect(chips.get("dddddddd")).toMatchObject({ state: "failed", label: "timed out" });
    expect(chips.get("eeeeeeee")).toMatchObject({ state: "refused", label: "not run", detail: "the run window passed" });
    // A command that could not start (a launcher failure) is recorded the
    // same way and reads the same: never "approved" (EYES 47818935).
    const unstarted = gateChipsOf([approved("ffffffff-0000-4000-8000-000000000006", { run_refusal: "could not start: No such file or directory" })]);
    expect(unstarted.get("ffffffff")).toMatchObject({ state: "refused", label: "not run" });
  });

  it("drops a prefix two gates share — a chip never guesses", () => {
    const chips = gateChipsOf([
      row("12345678-0000-4000-8000-000000000001", "answered", "Approve"),
      row("12345678-0000-4000-8000-000000000002", "pending"),
    ]);
    expect(chips.has("12345678")).toBe(false);
  });

  it("shows automatic runs without inventing a user approval or rejection", () => {
    const chips = gateChipsOf([
      row("aaaaaaaa-0000", "auto_running"),
      { ...row("bbbbbbbb-0000", "auto_resolved"), exit_code: 0 },
      { ...row("cccccccc-0000", "auto_resolved"), exit_code: 2 },
      { ...row("dddddddd-0000", "auto_resolved"), run_refusal: "body changed" },
    ]);
    expect(chips.get("aaaaaaaa")).toMatchObject({ state: "automatic", label: "auto-allowed" });
    expect(chips.get("bbbbbbbb")).toMatchObject({ state: "ran", label: "ran" });
    expect(chips.get("cccccccc")).toMatchObject({ state: "failed", label: "exit 2" });
    expect(chips.get("dddddddd")).toMatchObject({ state: "refused", label: "not run" });
  });

  it("recognises the quoted shape only", () => {
    expect(looksLikeGateId("236473a7")).toBe(true);
    expect(looksLikeGateId("236473A7")).toBe(true);
    expect(looksLikeGateId("236473a")).toBe(false);
    expect(looksLikeGateId("236473a7b")).toBe(false);
    expect(looksLikeGateId("cargo test")).toBe(false);
  });
});
