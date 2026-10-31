import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { ActionItemsPanel } from "./ActionItemsPanel";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockInvoke = vi.mocked(invoke);

const item = {
  session_id: "s-123f01aa",
  session_title: "project EOD",
  session_closed_at: "2026-10-02T10:00:00Z",
  slug: "eod-draft",
  updated_at: new Date(Date.now() - 5 * 60_000).toISOString(),
  last_writer: "hands",
  preview: "Today we shipped the thing.",
  bytes: 812,
};

function renderPanel(items: unknown[]) {
  mockInvoke.mockImplementation((cmd: string) => {
    if (cmd === "list_user_action_items") return Promise.resolve(items);
    if (cmd === "dismiss_user_action_item") return Promise.resolve(true);
    return Promise.resolve([]);
  });
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={["/"]}>
        <Routes>
          <Route path="/" element={<ActionItemsPanel />} />
          <Route path="/sessions/:id" element={<div data-testid="session-route">{window.location.search}</div>} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

// Feedback #89: the documents agents left for the user, across sessions,
// with Open and Dismiss; nothing renders when there are none.
describe("ActionItemsPanel", () => {
  beforeEach(() => mockInvoke.mockReset());

  it("lists an item from a closed session with its preview, and dismisses it", async () => {
    renderPanel([item]);
    const panel = await screen.findByTestId("action-items");
    expect(panel).toHaveTextContent("Your action items");
    expect(panel).toHaveTextContent("eod-draft");
    expect(panel).toHaveTextContent("project EOD · closed");
    expect(panel).toHaveTextContent("Today we shipped the thing.");
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("dismiss_user_action_item", {
        sessionId: "s-123f01aa",
        slug: "eod-draft",
      }),
    );
  });

  it("opens the session with the document named", async () => {
    renderPanel([item]);
    fireEvent.click(await screen.findByRole("button", { name: "Open" }));
    expect(await screen.findByTestId("session-route")).toBeInTheDocument();
  });

  it("renders nothing with no items", async () => {
    const { container } = renderPanel([]);
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("list_user_action_items", expect.anything()));
    expect(container.querySelector("[data-testid=action-items]")).toBeNull();
  });
});
