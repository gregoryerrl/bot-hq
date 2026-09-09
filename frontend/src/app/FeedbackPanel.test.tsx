import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { FeedbackPanel } from "./FeedbackPanel";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockInvoke = vi.mocked(invoke);

function row(over: Record<string, unknown> = {}) {
  return {
    id: 1,
    session_id: "s-224b28ce",
    project: "acme-data-ingest",
    agent: "rain",
    kind: "issue",
    title: "Gate command is unreadable",
    body: "The `--body-file` content never renders.",
    status: "open",
    created_at: "2026-07-28T12:00:00Z",
    updated_at: "2026-07-28T12:00:00Z",
    ...over,
  };
}

function renderPanel(rows: unknown[]) {
  mockInvoke.mockImplementation((cmd: string) => {
    if (cmd === "list_agent_feedback") return Promise.resolve(rows);
    return Promise.resolve(true);
  });
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>
        <FeedbackPanel />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("FeedbackPanel", () => {
  beforeEach(() => {
    mockInvoke.mockClear();
    mockInvoke.mockImplementation(() => Promise.resolve([]));
  });

  it("defaults to the open queue", async () => {
    renderPanel([row()]);
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("list_agent_feedback", {
        status: "open",
      }),
    );
    expect(
      await screen.findByText("Gate command is unreadable"),
    ).toBeInTheDocument();
  });

  it("shows provenance and body only once expanded", async () => {
    renderPanel([row()]);
    const title = await screen.findByText("Gate command is unreadable");
    expect(screen.queryByText(/acme-data-ingest/)).not.toBeInTheDocument();

    fireEvent.click(title);
    // Provenance = where the friction was hit, linked back to that session.
    expect(await screen.findByText(/acme-data-ingest/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "s-224b28" })).toHaveAttribute(
      "href",
      "/sessions/s-224b28ce",
    );
  });

  it("marks an item done", async () => {
    renderPanel([row()]);
    fireEvent.click(await screen.findByText("Gate command is unreadable"));
    fireEvent.click(await screen.findByRole("button", { name: "Mark done" }));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("set_agent_feedback_status", {
        id: 1,
        status: "done",
      }),
    );
  });

  it("switching to all clears the status filter", async () => {
    renderPanel([row()]);
    await screen.findByText("Gate command is unreadable");
    fireEvent.click(screen.getByRole("button", { name: "all" }));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("list_agent_feedback", {
        status: null,
      }),
    );
  });

  it("says how items get here when the queue is empty", async () => {
    renderPanel([]);
    expect(await screen.findByText(/file_feedback/)).toBeInTheDocument();
  });
});
