import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { EditorArea } from "./ContextLibraryEditor";
import type { OpenTab } from "./contextLibraryShared";
import { invoke } from "@tauri-apps/api/core";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockInvoke = vi.mocked(invoke);

function renderEditor(
  tab: OpenTab = { kind: "file", project: "p", filePath: "a.md" },
  onProjectChanged = vi.fn(),
) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const utils = render(
    <QueryClientProvider client={qc}>
      <EditorArea
        tabs={[tab]}
        activeTabIndex={0}
        onSelectTab={() => {}}
        onCloseTab={() => {}}
        activeTab={tab}
        entries={[]}
        folders={[]}
        projects={[]}
        onRefetchIndex={() => {}}
        onRefetchFolders={() => {}}
        onProjectChanged={onProjectChanged}
        onProjectGone={() => {}}
      />
    </QueryClientProvider>,
  );
  return { qc, ...utils };
}

describe("Context Library editor", () => {
  beforeEach(() => mockInvoke.mockReset());

  it("edits file content and saves it via cl_write_file", async () => {
    // Stateful mock: cl_write_file updates what the next cl_read_file returns,
    // mirroring the real round-trip so the dirty badge clears after save.
    let stored = "hello\nworld";
    mockInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === "cl_read_file") {
        return {
          project: "p",
          file_path: "a.md",
          content: stored,
          size_bytes: stored.length,
          truncated: false,
          binary: false,
        };
      }
      if (cmd === "cl_write_file") {
        stored = (args as { content: string }).content;
        return undefined;
      }
      return [];
    });

    renderEditor();

    const textarea = await screen.findByLabelText("File content editor");
    expect(textarea).toHaveValue("hello\nworld");
    // Clean file → nothing to save.
    expect(
      screen.getByRole("button", { name: /save changes/i }),
    ).toBeDisabled();

    fireEvent.change(textarea, { target: { value: "hello\nworld!" } });

    expect(await screen.findByText("UNSAVED CHANGES")).toBeInTheDocument();
    const save = screen.getByRole("button", { name: /save changes/i });
    expect(save).toBeEnabled();

    fireEvent.click(save);

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("cl_write_file", {
        project: "p",
        filePath: "a.md",
        content: "hello\nworld!",
      }),
    );
    // Baseline catches up on refetch → dirty indicator goes away.
    await waitFor(() =>
      expect(screen.queryByText("UNSAVED CHANGES")).not.toBeInTheDocument(),
    );
  });

  it("live-refreshes an open file when clean, but preserves unsaved edits", async () => {
    let stored = "v1";
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "cl_read_file") {
        return {
          project: "p",
          file_path: "a.md",
          content: stored,
          size_bytes: stored.length,
          truncated: false,
          binary: false,
        };
      }
      return [];
    });

    const { qc } = renderEditor();
    const textarea = await screen.findByLabelText("File content editor");
    expect(textarea).toHaveValue("v1");

    // External change while the editor is CLEAN → adopt the new content.
    stored = "v2 external";
    await qc.invalidateQueries({ queryKey: ["cl_read_file"] });
    await waitFor(() => expect(textarea).toHaveValue("v2 external"));
    expect(screen.queryByText("UNSAVED CHANGES")).not.toBeInTheDocument();

    // Type unsaved edits, then an external change → keep the user's text.
    fireEvent.change(textarea, { target: { value: "my local edits" } });
    expect(await screen.findByText("UNSAVED CHANGES")).toBeInTheDocument();

    const readsBefore = mockInvoke.mock.calls.filter(
      (c) => c[0] === "cl_read_file",
    ).length;
    stored = "v3 external";
    await qc.invalidateQueries({ queryKey: ["cl_read_file"] });
    await waitFor(() =>
      expect(
        mockInvoke.mock.calls.filter((c) => c[0] === "cl_read_file").length,
      ).toBeGreaterThan(readsBefore),
    );
    // The dirty editor must NOT be clobbered by the external change.
    expect(textarea).toHaveValue("my local edits");
    expect(screen.getByText("UNSAVED CHANGES")).toBeInTheDocument();
  });

  it("is read-only for binary files and blocks saving", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "cl_read_file") {
        return {
          project: "p",
          file_path: "a.md",
          content: "��",
          size_bytes: 2,
          truncated: false,
          binary: true,
        };
      }
      return [];
    });

    renderEditor();

    const textarea = await screen.findByLabelText("File content editor");
    expect(textarea).toHaveAttribute("readonly");
    expect(screen.getByText("READ-ONLY")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /save changes/i }),
    ).toBeDisabled();
  });

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

    renderEditor({ kind: "proposals", project: "p" }, onProjectChanged);

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

    renderEditor({ kind: "proposals", project: "p" }, onProjectChanged);

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

    renderEditor({ kind: "proposals", project: "p" });

    expect(await screen.findByText("old.md")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /approve unsupported/i })).toBeDisabled();
    expect(screen.getByText(/delete approval is deferred/i)).toBeInTheDocument();
  });
});

describe("Context Library measurement", () => {
  beforeEach(() => mockInvoke.mockReset());

  const STATS = {
    event_count: 12,
    distinct_sessions: 4,
    total_tokens: 6000,
    total_atoms: 30,
    stale_hits: 3,
    empty_returns: 2,
    avg_tokens_per_event: 500,
    avg_tokens_per_session: 1500,
    stale_hit_rate: 0.1,
    empty_return_rate: 0.1667,
  };

  it("renders retrieval telemetry from cl_retrieval_stats", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "cl_retrieval_stats") return STATS;
      return null;
    });

    renderEditor({ kind: "measurement", project: "p" });

    // Await a data-dependent tile so the query has resolved past "Loading…".
    // Locale-independent assertions: the toFixed rates, not comma-grouped ints.
    expect(await screen.findByText("10.0%")).toBeInTheDocument(); // stale-hit rate
    expect(screen.getByText("Retrieval measurement")).toBeInTheDocument();
    expect(screen.getByText("Tokens / session")).toBeInTheDocument();
    expect(screen.getByText("16.7%")).toBeInTheDocument(); // retrieval-miss rate
  });

  it("shows an empty state when no retrievals are logged", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "cl_retrieval_stats") return { ...STATS, event_count: 0 };
      return null;
    });

    renderEditor({ kind: "measurement", project: "p" });

    expect(
      await screen.findByText("No retrievals logged yet"),
    ).toBeInTheDocument();
  });
});
