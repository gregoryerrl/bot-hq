import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { Markdown } from "./Markdown";
import { GateChipsContext, gateChipsOf } from "../lib/gateChips";

describe("Markdown", () => {
  it("renders headings, links, and lists from markdown source", () => {
    render(
      <Markdown>
        {"# Title\n\nSee [docs](https://example.com)\n\n- one\n- two"}
      </Markdown>,
    );
    expect(screen.getByRole("heading", { name: "Title" })).toBeInTheDocument();
    const link = screen.getByRole("link", { name: "docs" });
    expect(link).toHaveAttribute("href", "https://example.com");
    expect(link).toHaveAttribute("target", "_blank");
    expect(screen.getByText("one")).toBeInTheDocument();
    expect(screen.getByText("two")).toBeInTheDocument();
  });

  it("renders inline code without crashing", () => {
    render(<Markdown>{"use `cargo test` to run"}</Markdown>);
    expect(screen.getByText("cargo test")).toBeInTheDocument();
  });
});

// Feedback #61: "waiting on your approval of `236473a7`" reads as stale right
// where it is written once the gate has been approved — a chip beside the
// token; a token that resolves to no gate of this session (a short sha) and a
// Markdown outside a session get none.
describe("gate chips in chat markdown", () => {
  const chips = gateChipsOf([
    { choice_id: "236473a7-0000-4000-8000-000000000001", status: "answered", command_text: "gh pr merge 5", picked_option: "Approve" },
    { choice_id: "c2d52757-0000-4000-8000-000000000003", status: "pending", command_text: "psql -h prod -c x", picked_option: null },
  ]);

  it("marks a resolving token with the gate's state and leaves the rest alone", () => {
    render(
      <GateChipsContext.Provider value={chips}>
        <Markdown>{"Waiting on your approval of `236473a7`; also `c2d52757` is parked; commit `9a07930f` is unrelated."}</Markdown>
      </GateChipsContext.Provider>,
    );
    const marks = screen.getAllByTestId("gate-chip");
    expect(marks.map((m) => m.textContent)).toEqual(["approved", "pending"]);
    expect(marks[0]).toHaveAttribute("title", "gate 236473a7-0000-4000-8000-000000000001: approved");
  });

  it("shows nothing outside a session", () => {
    render(<Markdown>{"gate `236473a7` was approved"}</Markdown>);
    expect(screen.queryByTestId("gate-chip")).not.toBeInTheDocument();
  });
});
