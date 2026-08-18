import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { ClosedSessionBar } from "./ClosedSessionBar";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const invokeMock = vi.mocked(invoke);

describe("ClosedSessionBar — a closed session reopens on a button, not on a view", () => {
  beforeEach(() => invokeMock.mockReset());

  it("says the history is read-only and offers exactly one Reopen", () => {
    render(<ClosedSessionBar sessionId="s-1" closedAt="2026-08-18T04:50:41Z" />);
    expect(screen.getByRole("status", { name: "Session closed" })).toHaveTextContent(
      /history is read-only/,
    );
    expect(screen.getAllByRole("button", { name: "Reopen" })).toHaveLength(1);
    // Nothing spawns on render — that is the whole point of the bar.
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("clicking Reopen invokes reopen_session for THIS session", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    render(<ClosedSessionBar sessionId="s-1" closedAt="2026-08-18T04:50:41Z" />);
    fireEvent.click(screen.getByRole("button", { name: "Reopen" }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("reopen_session", { sessionId: "s-1" }),
    );
    // The view flips through the invalidated get_session read; the bar's own
    // job ends with the call, so it is enabled again and shows no error.
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Reopen" })).not.toBeDisabled(),
    );
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("surfaces a refused reopen inline instead of swallowing it", async () => {
    invokeMock.mockRejectedValueOnce({ message: "session s-1 is not closed; nothing to reopen" });
    render(<ClosedSessionBar sessionId="s-1" closedAt="2026-08-18T04:50:41Z" />);
    fireEvent.click(screen.getByRole("button", { name: "Reopen" }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/Reopen failed:/);
    expect(alert).toHaveTextContent(/not closed/);
  });
});
