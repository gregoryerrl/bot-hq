import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { PhasePill, PhasePillRow } from "./PhasePill";

describe("PhasePill", () => {
  it("renders the single-letter label", () => {
    const onSelect = vi.fn();
    render(<PhasePill phase="apply" selected={true} onSelect={onSelect} />);
    expect(screen.getByText("A")).toBeInTheDocument();
  });

  it("calls onSelect when clicked", () => {
    const onSelect = vi.fn();
    render(<PhasePill phase="plan" selected={false} onSelect={onSelect} />);
    fireEvent.click(screen.getByText("P"));
    expect(onSelect).toHaveBeenCalledWith("plan");
  });

  // Round 12: the phase tint belongs to the SELECTED pill only. `cn` is clsx,
  // so an unselected pill carrying both the tint and the muted colour was
  // settled by stylesheet order — and the tint came later in the built CSS.
  it("tints only the selected pill; an unselected one is muted, with no tint class to fight it", () => {
    const { rerender } = render(<PhasePill phase="plan" selected={true} onSelect={() => {}} />);
    const selectedClasses = screen.getByRole("tab").className;
    expect(selectedClasses).toContain("text-primary");
    expect(selectedClasses).not.toContain("text-on-surface-variant");
    rerender(<PhasePill phase="plan" selected={false} onSelect={() => {}} />);
    const unselected = screen.getByRole("tab").className;
    expect(unselected).toContain("text-on-surface-variant");
    expect(unselected).not.toContain("text-primary");
    expect(unselected).not.toContain("border-primary");
  });
});

describe("PhasePillRow", () => {
  it("renders all four IPAV pills", () => {
    render(<PhasePillRow selected="investigate" onSelect={() => {}} />);
    expect(screen.getByText("I")).toBeInTheDocument();
    expect(screen.getByText("P")).toBeInTheDocument();
    expect(screen.getByText("A")).toBeInTheDocument();
    expect(screen.getByText("V")).toBeInTheDocument();
  });
});
