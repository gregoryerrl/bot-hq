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
    // The filed-by label reads the filing session's roster.
    if (cmd === "list_session_participants") return Promise.resolve([]);
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

  it("names the filer from the filing session's roster, never by slug", async () => {
    mockInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "list_agent_feedback") return Promise.resolve([row({ agent: "eyes" })]);
      if (cmd === "list_session_participants") {
        const { sessionId } = (args ?? {}) as { sessionId?: string };
        return Promise.resolve(
          sessionId === "s-224b28ce"
            ? [
                {
                  id: 2,
                  slug: "eyes",
                  role_display_name: "EYES",
                  model_display_name: "DeepSeek V4 Pro",
                  turn_position: 1,
                  participation_mode: "active",
                  enabled: true,
                },
              ]
            : [],
        );
      }
      return Promise.resolve(true);
    });
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={qc}>
        <MemoryRouter>
          <FeedbackPanel />
        </MemoryRouter>
      </QueryClientProvider>,
    );
    expect(await screen.findByText(/EYES · DeepSeek V4 Pro/)).toBeInTheDocument();
    expect(screen.queryByText(/^eyes ·/)).not.toBeInTheDocument();
  });

  it("shows a placeholder while loading and the empty copy only once the queue answered", async () => {
    let resolve: (rows: unknown[]) => void = () => {};
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "list_agent_feedback") return new Promise((r) => (resolve = r));
      return Promise.resolve([]);
    });
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={qc}>
        <MemoryRouter>
          <FeedbackPanel />
        </MemoryRouter>
      </QueryClientProvider>,
    );
    expect(screen.queryByText(/Nothing open/)).not.toBeInTheDocument();
    resolve([]);
    expect(await screen.findByText(/Nothing open/)).toBeInTheDocument();
  });

  it("shows why a closed row was closed, and who closed it", async () => {
    renderPanel([
      row({
        id: 7,
        status: "dismissed",
        resolution: "declined by the user in s-079dec53",
        resolved_by: "hands \u00b7 s-57224745",
      }),
      row({ id: 8, title: "Still open", status: "open", resolution: null, resolved_by: null }),
    ]);
    fireEvent.click(await screen.findByRole("button", { name: "All" }));
    expect(await screen.findByText(/declined by the user in s-079dec53/)).toBeInTheDocument();
    expect(screen.getByText("Dismissed:")).toBeInTheDocument();
    expect(screen.queryByText(/hands · s-57224745/)).not.toBeInTheDocument();
    expect(screen.getAllByText(/Dismissed:|Done:/)).toHaveLength(1);
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
    fireEvent.click(screen.getByRole("button", { name: "All" }));
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
