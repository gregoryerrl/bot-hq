import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { MemoryRouter } from "react-router-dom";
import { SessionTile } from "./SessionTile";
import type { PendingChoiceView, SessionInfo } from "../lib/bindings";

const session: SessionInfo = {
  id: "s-abcd1234",
  title: "Refactor auth flow",
  working_repo_path: null,
  archived: false,
  created_at: "2026-05-26T18:00:00Z",
  closed_at: null,
  brian_model_at_spawn: null,
  rain_model_at_spawn: null,
};

const binaryChoice: PendingChoiceView = {
  choice_id: "c1",
  session_id: session.id,
  agent: "brian",
  question: "Approve the planned override?",
  options: ["Approve", "Reject"],
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

  it("renders the [Need User Input] pill when pendingChoices is non-empty", () => {
    renderTile({ pendingChoices: [binaryChoice] });
    expect(screen.getByText(/need user input/i)).toBeInTheDocument();
  });

  it("hides the [Need User Input] pill when no pending choices", () => {
    renderTile();
    expect(screen.queryByText(/need user input/i)).not.toBeInTheDocument();
  });

  it("renders the inline question and option buttons for a pending choice", () => {
    renderTile({ pendingChoices: [binaryChoice] });
    expect(screen.getByText(/approve the planned override/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^approve$/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^reject$/i })).toBeInTheDocument();
  });
});
