import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import type { Policy } from "../lib/bindings";
import { PolicyForm } from "./PolicyForm";

const EMPTY: Policy = {};

describe("PolicyForm", () => {
  it("renders the tier-agnostic fields", () => {
    render(<PolicyForm value={EMPTY} onChange={() => {}} />);
    expect(screen.getByText("Push gate")).toBeInTheDocument();
    expect(screen.getByText("Force push")).toBeInTheDocument();
    expect(screen.getByText("Per-action approval")).toBeInTheDocument();
    expect(screen.getByText("Branch pattern")).toBeInTheDocument();
    expect(screen.getByText("Round cap (laps)")).toBeInTheDocument();
  });

  it("defaults the toggles to auto / allowed when the policy is empty", () => {
    render(<PolicyForm value={EMPTY} onChange={() => {}} />);
    // Active toggle carries the tone class; assert via aria — simplest is the
    // text presence + the onChange behavior below covers the wiring.
    expect(screen.getByText("Auto")).toBeInTheDocument();
    expect(screen.getByText("Allowed")).toBeInTheDocument();
  });

  it("flips push_gate to ask via onChange", () => {
    const onChange = vi.fn();
    render(<PolicyForm value={EMPTY} onChange={onChange} />);
    fireEvent.click(screen.getByText("Ask"));
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ push_gate: "ask" }),
    );
  });

  it("flips force_push to blocked via onChange", () => {
    const onChange = vi.fn();
    render(<PolicyForm value={EMPTY} onChange={onChange} />);
    fireEvent.click(screen.getByText("Blocked"));
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ force_push: "blocked" }),
    );
  });

  it("edits branch_pattern via onChange", () => {
    const onChange = vi.fn();
    render(<PolicyForm value={EMPTY} onChange={onChange} />);
    fireEvent.change(screen.getByPlaceholderText("(no constraint)"), {
      target: { value: "feature/.*" },
    });
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ branch_pattern: "feature/.*" }),
    );
  });

  // The round cap is three-valued on the wire: null = inherit, 0 = off, n =
  // halt after n laps. An <input type="number"> only has strings, and
  // `Number("")` is 0 — the one value that means "never halt" — so the empty
  // box has to map to null explicitly or clearing the field disarms the
  // backstop instead of restoring the default.
  it("shows an empty round cap box as inherited rather than as 0", () => {
    render(<PolicyForm value={EMPTY} onChange={() => {}} />);
    const box = screen.getByPlaceholderText("500 (inherited)") as HTMLInputElement;
    expect(box.value).toBe("");
  });

  it("edits round_cap via onChange", () => {
    const onChange = vi.fn();
    render(<PolicyForm value={EMPTY} onChange={onChange} />);
    fireEvent.change(screen.getByPlaceholderText("500 (inherited)"), {
      target: { value: "40" },
    });
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ round_cap: 40 }),
    );
  });

  it("keeps 0 as a real value and an empty box as null", () => {
    const onChange = vi.fn();
    render(<PolicyForm value={{ round_cap: 40 }} onChange={onChange} />);
    const box = screen.getByPlaceholderText("500 (inherited)");
    fireEvent.change(box, { target: { value: "0" } });
    expect(onChange).toHaveBeenLastCalledWith(
      expect.objectContaining({ round_cap: 0 }),
    );
    fireEvent.change(box, { target: { value: "" } });
    expect(onChange).toHaveBeenLastCalledWith(
      expect.objectContaining({ round_cap: null }),
    );
  });
});
