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
