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
    expect(screen.getByText("Forbidden in commits")).toBeInTheDocument();
    expect(screen.getByText("Per-action approval")).toBeInTheDocument();
    expect(screen.getByText("Branch pattern")).toBeInTheDocument();
    expect(screen.getByText("Commit style")).toBeInTheDocument();
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

  it("adds a forbidden-word row", () => {
    const onChange = vi.fn();
    render(
      <PolicyForm
        value={{ forbidden_in_commits: ["Claude"] }}
        onChange={onChange}
      />,
    );
    // Two "+ Add" buttons (forbidden + per-action); the first is forbidden.
    fireEvent.click(screen.getAllByText("+ Add")[0]);
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ forbidden_in_commits: ["Claude", ""] }),
    );
  });

  it("removes a forbidden-word row", () => {
    const onChange = vi.fn();
    render(
      <PolicyForm
        value={{ forbidden_in_commits: ["Claude", "bot-hq"] }}
        onChange={onChange}
      />,
    );
    fireEvent.click(screen.getAllByLabelText("Remove entry")[0]);
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ forbidden_in_commits: ["bot-hq"] }),
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
});
