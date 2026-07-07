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

function proposal(overrides: Record<string, unknown> = {}) {
  return {
    id: 1,
    proposal_uid: "p1",
    project: "p",
    file_path: "notes.md",
    kind: "correct",
    target_excerpt: null,
    proposed_body: "complete corrected body",
    evidence: "stale wording",
    status: "open",
    proposed_by: "rain",
    session_id: "s1",
    created_at: "2026-06-29T00:00:00Z",
    updated_at: "2026-06-29T00:00:00Z",
    conflict: null,
    open_siblings: 0,
    ...overrides,
  };
}

describe("ProposalQueue", () => {
  beforeEach(() => mockInvoke.mockReset());

  it("renders proposal docket cards and rejects proposals", async () => {
    const onProjectChanged = vi.fn();
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "cl_list_proposals") {
        return [proposal({ target_excerpt: "old wording" })];
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

  it("approves clean proposals without force and refreshes the CL workspace", async () => {
    const onProjectChanged = vi.fn();
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "cl_list_proposals") {
        return [
          proposal({
            proposal_uid: "p2",
            file_path: "new.md",
            kind: "add",
            proposed_body: "new file body",
            evidence: "missing note",
            proposed_by: "brian",
          }),
        ];
      }
      if (cmd === "cl_approve_proposal") return "proposal 'p2' approved";
      return [];
    });

    renderQueue(onProjectChanged);

    expect(await screen.findByText("new.md")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /^approve$/i }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("cl_approve_proposal", {
        proposalUid: "p2",
        force: false,
      }),
    );
    await waitFor(() => expect(onProjectChanged).toHaveBeenCalled());
  });

  it("offers delete approval with a destructive warning", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "cl_list_proposals") {
        return [
          proposal({
            proposal_uid: "p3",
            file_path: "old.md",
            kind: "delete",
            proposed_body: "",
            evidence: "obsolete",
          }),
        ];
      }
      if (cmd === "cl_approve_proposal") return "proposal 'p3' approved";
      return [];
    });

    renderQueue();

    expect(await screen.findByText("old.md")).toBeInTheDocument();
    expect(screen.getByText(/permanently deletes this file/i)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /delete file/i }));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("cl_approve_proposal", {
        proposalUid: "p3",
        force: false,
      }),
    );
  });

  it("labels a conflicted proposal and approves it with force", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "cl_list_proposals") {
        return [
          proposal({
            proposal_uid: "p4",
            file_path: "notes.md",
            kind: "add",
            proposed_body: "colliding body",
            evidence: "filed before the file appeared",
            conflict: "exists",
          }),
        ];
      }
      if (cmd === "cl_approve_proposal") return "proposal 'p4' approved";
      return [];
    });

    renderQueue();

    expect(await screen.findByText("FILE EXISTS")).toBeInTheDocument();
    expect(screen.getByText(/now exists in the CL/i)).toBeInTheDocument();

    fireEvent.click(
      screen.getByRole("button", { name: /replace existing file/i }),
    );
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("cl_approve_proposal", {
        proposalUid: "p4",
        force: true,
      }),
    );
  });

  it("surfaces competing open proposals on the same file", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "cl_list_proposals") {
        return [proposal({ open_siblings: 2 })];
      }
      return [];
    });

    renderQueue();

    expect(
      await screen.findByText(/2 other open proposals target this file/i),
    ).toBeInTheDocument();
  });

  it("compares a correct proposal against the current file", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "cl_list_proposals") {
        return [proposal({ proposed_body: "kept line\nnew line" })];
      }
      if (cmd === "cl_read_file") {
        return {
          project: "p",
          file_path: "notes.md",
          content: "kept line\nold line",
          size_bytes: 18,
          truncated: false,
          binary: false,
        };
      }
      return [];
    });

    renderQueue();

    fireEvent.click(
      await screen.findByRole("button", { name: /compare with current file/i }),
    );

    expect(await screen.findByText("- old line")).toBeInTheDocument();
    expect(screen.getByText("+ new line")).toBeInTheDocument();
    expect(
      screen.getByText("  kept line", { normalizer: (s) => s }),
    ).toBeInTheDocument();
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("cl_read_file", {
        project: "p",
        filePath: "notes.md",
      }),
    );
  });
});
