import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { MemoryRouter } from "react-router-dom";
import { SessionTile } from "./SessionTile";
import type { SessionInfo } from "../lib/bindings";

const session: SessionInfo = {
  id: "s1",
  title: "Refactor auth flow",
  working_repo_path: null,
  archived: false,
  created_at: "2026-05-26T18:00:00Z",
  closed_at: null,
  brian_model_at_spawn: null,
  rain_model_at_spawn: null,
};

describe("SessionTile", () => {
  it("shows the title and links to session view", () => {
    render(
      <MemoryRouter>
        <SessionTile session={session} />
      </MemoryRouter>,
    );
    const link = screen.getByRole("link");
    expect(link).toHaveAttribute("href", "/sessions/s1");
    expect(screen.getByText(/Refactor auth flow/i)).toBeInTheDocument();
  });

  it("renders Needs Input badge when needsInput is true", () => {
    render(
      <MemoryRouter>
        <SessionTile session={session} needsInput />
      </MemoryRouter>,
    );
    expect(screen.getByText(/needs input/i)).toBeInTheDocument();
  });

  it("hides Needs Input badge by default", () => {
    render(
      <MemoryRouter>
        <SessionTile session={session} />
      </MemoryRouter>,
    );
    expect(screen.queryByText(/needs input/i)).not.toBeInTheDocument();
  });
});
