import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { MemoryRouter } from "react-router-dom";
import { SessionTile } from "./SessionTile";
import type { SessionInfo } from "../lib/bindings";

const session: SessionInfo = {
  id: "s-abcd1234",
  title: "Refactor auth flow",
  working_repo_path: null,
  base_repo_path: null,
  archived: false,
  created_at: "2026-05-26T18:00:00Z",
  closed_at: null,
  brian_model_at_spawn: null,
  rain_model_at_spawn: null,
  rain_enabled: true,
  last_message: null,
  last_author: null,
};

function renderTile(props: Partial<React.ComponentProps<typeof SessionTile>> = {}) {
  return render(
    <MemoryRouter>
      <SessionTile session={session} {...props} />
    </MemoryRouter>,
  );
}

describe("SessionTile", () => {
  it("renders the title and a navigable role=link surface", () => {
    renderTile();
    const tile = screen.getByRole("link", { name: /refactor auth flow/i });
    expect(tile).toBeInTheDocument();
    expect(tile).toHaveAttribute("tabindex", "0");
    expect(screen.getByText(/Refactor auth flow/i)).toBeInTheDocument();
  });

  it("shows the session id chip in S-XXXX form", () => {
    renderTile();
    expect(screen.getByText("S-ABCD")).toBeInTheDocument();
  });

  it("renders the [Need User Input] pill when pendingCount > 0", () => {
    renderTile({ pendingCount: 1 });
    expect(screen.getByText(/need user input/i)).toBeInTheDocument();
  });

  it("hides the [Need User Input] pill when pendingCount is 0", () => {
    renderTile();
    expect(screen.queryByText(/need user input/i)).not.toBeInTheDocument();
  });

  it("indicates pending input without an inline answer surface", () => {
    // The tile only INDICATES a count; the question + options live on the Tray tab.
    renderTile({ pendingCount: 1 });
    expect(
      screen.queryByRole("button", { name: /^approve$/i }),
    ).not.toBeInTheDocument();
  });

  it("shows the pending count in the indicator", () => {
    renderTile({ pendingCount: 2 });
    expect(screen.getByText(/need user input · 2/i)).toBeInTheDocument();
  });

  it("shows the first line of the latest message + author tag in Quickview", () => {
    renderTile({
      session: {
        ...session,
        last_message: "Looking at the storage layer now\nsecond line",
        last_author: "brian",
      },
    });
    expect(screen.getByText("Brian")).toBeInTheDocument();
    expect(
      screen.getByText(/looking at the storage layer now/i),
    ).toBeInTheDocument();
    // Only the first line surfaces; trailing lines are dropped.
    expect(screen.queryByText(/second line/i)).not.toBeInTheDocument();
  });

  it("labels a user-authored latest message as You", () => {
    renderTile({
      session: {
        ...session,
        last_message: "Fix the login bug",
        last_author: "user",
      },
    });
    expect(screen.getByText("You")).toBeInTheDocument();
    expect(screen.getByText(/fix the login bug/i)).toBeInTheDocument();
  });

  it("falls back to a generic Quickview hint when there is no message", () => {
    renderTile({ phase: "investigate" });
    expect(
      screen.getByText(/open to view activity/i),
    ).toBeInTheDocument();
  });
});
