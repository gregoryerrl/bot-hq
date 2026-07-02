import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ProposalQueue } from "./ProposalQueue";
import { invoke } from "@tauri-apps/api/core";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockInvoke = vi.mocked(invoke);

function renderQueue(onProjectChanged = vi.fn()) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const utils = render(
    <QueryClientProvider client={qc}>
      <ProposalQueue project="p" onProjectChanged={onProjectChanged} />
    </QueryClientProvider>,
  );
  return { qc, ...utils };
}

describe("ProposalQueue", () => {
  beforeEach(() => mockInvoke.mockReset());

  it("renders proposal docket cards and rejects proposals", async () => {
    const onProjectChanged = vi.fn();
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "cl_list_proposals") {
        return [
          {
            id: 1,
            proposal_uid: "p1",
            project: "p",
            file_path: "notes.md",
            kind: "correct",
            target_excerpt: "old wording",
            proposed_body: "complete corrected body",
            evidence: "stale wording",
            status: "open",
            proposed_by: "rain",
            session_id: "s1",
            created_at: "2026-06-29T00:00:00Z",
            updated_at: "2026-06-29T00:00:00Z",
          },
        ];
      }
      if (cmd === "cl_reject_proposal") return "proposal 'p1' rejected";
      return [];
    });

    renderQueue(onProjectChanged);

    expect(await screen.findByText("notes.md")).toBeInTheDocument();
    expect(screen.getByText("stale wording")).toBeInTheDocument();
    expect(screen.getByText("complete corrected body")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /reject proposal/i }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("cl_reject_proposal", {
        proposalUid: "p1",
      }),
    );
    await waitFor(() => expect(onProjectChanged).toHaveBeenCalled());
  });

  it("approves supported proposals and refreshes the CL workspace", async () => {
    const onProjectChanged = vi.fn();
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "cl_list_proposals") {
        return [
          {
            id: 1,
            proposal_uid: "p2",
            project: "p",
            file_path: "new.md",
            kind: "add",
            target_excerpt: null,
            proposed_body: "new file body",
            evidence: "missing note",
            status: "open",
            proposed_by: "brian",
            session_id: "s1",
            created_at: "2026-06-29T00:00:00Z",
            updated_at: "2026-06-29T00:00:00Z",
          },
        ];
      }
      if (cmd === "cl_approve_proposal") return "proposal 'p2' approved";
      return [];
    });

    renderQueue(onProjectChanged);

    expect(await screen.findByText("new.md")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /approve proposal/i }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("cl_approve_proposal", {
        proposalUid: "p2",
      }),
    );
    await waitFor(() => expect(onProjectChanged).toHaveBeenCalled());
  });

  it("marks delete proposal approval as unsupported", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "cl_list_proposals") {
        return [
          {
            id: 1,
            proposal_uid: "p3",
            project: "p",
            file_path: "old.md",
            kind: "delete",
            target_excerpt: null,
            proposed_body: "",
            evidence: "obsolete",
            status: "open",
            proposed_by: "rain",
            session_id: "s1",
            created_at: "2026-06-29T00:00:00Z",
            updated_at: "2026-06-29T00:00:00Z",
          },
        ];
      }
      return [];
    });

    renderQueue();

    expect(await screen.findByText("old.md")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /approve unsupported/i }),
    ).toBeDisabled();
    expect(
      screen.getByText(/delete approval is deferred/i),
    ).toBeInTheDocument();
  });
});
