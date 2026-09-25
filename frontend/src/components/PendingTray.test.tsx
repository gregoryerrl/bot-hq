import { fireEvent, render, screen } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { PendingTray } from "./PendingTray";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockInvoke = vi.mocked(invoke);

// The user, 2026-09-25: "gates also don't show on notification bell". The bell
// now names questions, approval gates and halts — each kind on its own row
// phrase, the badge still counting SESSIONS.
describe("PendingTray — the notification bell", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "list_pending_tray") {
        return [
          // A gate-only session.
          { session_id: "s-gate0001", kind: "approval", options: ["Approve", "Reject"] },
          // A question.
          { session_id: "s-ask00001", kind: "choice", options: ["A", "B"] },
        ];
      }
      if (cmd === "list_session_halts") {
        return [
          // A halt-only session…
          {
            session_id: "s-halt0001",
            declared_by: "hands",
            reason: "First launch passed — your move",
            declared_at: "2026-09-15T04:18:58Z",
            wake_at: null,
          },
          // …and a temporary halt far in the future: waiting on something
          // external, so the bell leaves it out.
          {
            session_id: "s-wake0001",
            declared_by: "eyes",
            reason: "CI",
            declared_at: "2026-09-25T04:57:29Z",
            wake_at: "2999-01-01T00:00:00Z",
          },
        ];
      }
      return null;
    });
  });

  it("counts a gate-only session and a halt-only session, and names each kind", async () => {
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={qc}>
        <MemoryRouter>
          <PendingTray />
        </MemoryRouter>
      </QueryClientProvider>,
    );
    const bell = await screen.findByRole("button", { name: /notifications \(3 sessions need input\)/i });
    fireEvent.click(bell);
    expect(screen.getByText("approval waiting")).toBeInTheDocument();
    expect(screen.getByText("1 question")).toBeInTheDocument();
    expect(screen.getByText("halted — your move")).toBeInTheDocument();
    expect(screen.getByText("First launch passed — your move")).toBeInTheDocument();
    expect(screen.queryByText(/S-WAKE/i)).toBeNull();
  });
});
