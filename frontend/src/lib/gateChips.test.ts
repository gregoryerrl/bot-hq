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

  it("drops a prefix two gates share — a chip never guesses", () => {
    const chips = gateChipsOf([
      row("12345678-0000-4000-8000-000000000001", "answered", "Approve"),
      row("12345678-0000-4000-8000-000000000002", "pending"),
    ]);
    expect(chips.has("12345678")).toBe(false);
  });

  it("recognises the quoted shape only", () => {
    expect(looksLikeGateId("236473a7")).toBe(true);
    expect(looksLikeGateId("236473A7")).toBe(true);
    expect(looksLikeGateId("236473a")).toBe(false);
    expect(looksLikeGateId("236473a7b")).toBe(false);
    expect(looksLikeGateId("cargo test")).toBe(false);
  });
});
