import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { ClosedSessionBar } from "./ClosedSessionBar";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const invokeMock = vi.mocked(invoke);

function renderBar(qc = new QueryClient()) {
  return render(
    <QueryClientProvider client={qc}>
      <ClosedSessionBar sessionId="s-1" closedAt="2026-08-18T04:50:41Z" />
    </QueryClientProvider>,
  );
}

describe("ClosedSessionBar — a closed session reopens on a button, not on a view", () => {
  beforeEach(() => invokeMock.mockReset());

  it("says the history is read-only and offers exactly one Reopen", () => {
    renderBar();
    expect(screen.getByRole("status", { name: "Session closed" })).toHaveTextContent(
      /history is read-only/,
    );
    expect(screen.getAllByRole("button", { name: "Reopen" })).toHaveLength(1);
    // Nothing spawns on render — that is the whole point of the bar.
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("clicking Reopen invokes reopen_session for THIS session, then refetches the row", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    const qc = new QueryClient();
    const invalidate = vi.spyOn(qc, "invalidateQueries");
    renderBar(qc);
    fireEvent.click(screen.getByRole("button", { name: "Reopen" }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("reopen_session", { sessionId: "s-1" }),
    );
    // **The view flips through a REFETCHED `get_session` read, and this bar is
    // what asks for it** (round 11, issues.md 2026-08-18): the backend's
    // `session:created` only refreshed the dashboard list, so the closed view
    // stayed on screen after a successful reopen and the user's second click
    // errored "already open". The bar's own job ends with the call, so it is
    // enabled again and shows no error.
    await waitFor(() =>
      expect(invalidate).toHaveBeenCalledWith(
        expect.objectContaining({ queryKey: ["get_session", { sessionId: "s-1" }] }),
      ),
    );
    expect(invalidate).toHaveBeenCalledWith(
      expect.objectContaining({ queryKey: ["list_sessions"] }),
    );
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Reopen" })).not.toBeDisabled(),
    );
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("surfaces a refused reopen inline instead of swallowing it", async () => {
    invokeMock.mockRejectedValueOnce({ message: "spawning the roster failed" });
    renderBar();
    fireEvent.click(screen.getByRole("button", { name: "Reopen" }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/Reopen failed:/);
    expect(alert).toHaveTextContent(/spawning the roster failed/);
  });
});
