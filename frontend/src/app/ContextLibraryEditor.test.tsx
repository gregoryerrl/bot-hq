import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { useState } from "react";
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

  /** Two file tabs, rendered by the real EditorArea, with `activeTabIndex`
   *  controlled by the test so a switch is exactly what the app does. */
  function renderTwoTabs() {
    const files: Record<string, string> = { "a.md": "alpha", "b.md": "beta" };
    mockInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === "cl_read_file") {
        const fp = (args as { filePath: string }).filePath;
        return {
          project: "p",
          file_path: fp,
          content: files[fp] ?? "",
          size_bytes: (files[fp] ?? "").length,
          truncated: false,
          binary: false,
        };
      }
      return [];
    });
    const tabs: OpenTab[] = [
      { kind: "file", project: "p", filePath: "a.md" },
      { kind: "file", project: "p", filePath: "b.md" },
    ];
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const onCloseTab = vi.fn();
    function Harness() {
      const [active, setActive] = useState(0);
      return (
        <QueryClientProvider client={qc}>
          <EditorArea
            tabs={tabs}
            activeTabIndex={active}
            onSelectTab={setActive}
            onCloseTab={onCloseTab}
            entries={[]}
            folders={[]}
            projects={[]}
            onRefetchIndex={() => {}}
            onRefetchFolders={() => {}}
            onProjectChanged={() => {}}
            onProjectGone={() => {}}
          />
        </QueryClientProvider>
      );
    }
    return { ...render(<Harness />), onCloseTab };
  }

  it("keeps unsaved text when you switch tabs and come back", async () => {
    // The bug this pins: EditorArea rendered only the active tab, keyed by
    // path, so a switch UNMOUNTED the pane and the working copy — component
    // -local state — went with it. Type in A, open B, come back: gone, no
    // prompt. Now every open pane stays mounted and the inactive ones are
    // hidden, so nothing is restored and nothing can be lost.
    renderTwoTabs();

    // Both panes mount, but their reads resolve independently — wait for the
    // second rather than racing it.
    await waitFor(() =>
      expect(screen.getAllByLabelText("File content editor")).toHaveLength(2),
    );
    const a = screen.getAllByLabelText("File content editor")[0];
    expect(a).toHaveValue("alpha");
    fireEvent.change(a, { target: { value: "alpha EDITED" } });

    // Switch to B and back.
    fireEvent.click(screen.getByTitle("p — b.md"));
    fireEvent.click(screen.getByTitle("p — a.md"));

    expect(screen.getAllByLabelText("File content editor")[0]).toHaveValue(
      "alpha EDITED",
    );
  });

  it("marks the dirty tab and asks before closing it", async () => {
    // Two halves of one promise: the user can SEE which tab owes a save, and
    // the close button stops discarding it silently. Shipping the marker alone
    // would have been an invitation to lose text.
    const { onCloseTab } = renderTwoTabs();
    await waitFor(() =>
      expect(screen.getAllByLabelText("File content editor")).toHaveLength(2),
    );
    fireEvent.change(screen.getAllByLabelText("File content editor")[0], {
      target: { value: "alpha EDITED" },
    });

    expect(await screen.findByLabelText("Unsaved changes")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Close a\.md/ }));
    expect(onCloseTab).not.toHaveBeenCalled();
    // The dialog, not the tab marker — both say "unsaved changes", which is the
    // point: the marker warned, and the dialog is the thing that stops the loss.
    expect(screen.getByText(/closing discards them/i)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /discard and close/i }));
    expect(onCloseTab).toHaveBeenCalledWith(0);
  });

  it("forgets a closed tab's dirty state — reopening it is a clean tab", async () => {
    // The dirty set is keyed by `tabKey`, and a closed tab's key has to leave
    // it. If it lingers, reopening the same file shows a marker on a clean tab
    // and its close button prompts "discard" over nothing — a guard that cries
    // wolf is a guard the user learns to click through.
    const files: Record<string, string> = { "a.md": "alpha", "b.md": "beta" };
    mockInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === "cl_read_file") {
        const fp = (args as { filePath: string }).filePath;
        return {
          project: "p",
          file_path: fp,
          content: files[fp] ?? "",
          size_bytes: (files[fp] ?? "").length,
          truncated: false,
          binary: false,
        };
      }
      return [];
    });
    const a: OpenTab = { kind: "file", project: "p", filePath: "a.md" };
    const b: OpenTab = { kind: "file", project: "p", filePath: "b.md" };
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    function Harness() {
      const [tabs, setTabs] = useState<OpenTab[]>([a, b]);
      const [active, setActive] = useState(0);
      return (
        <QueryClientProvider client={qc}>
          <button type="button" onClick={() => { setTabs([a, b]); setActive(0); }}>
            reopen a.md
          </button>
          <EditorArea
            tabs={tabs}
            activeTabIndex={active}
            onSelectTab={setActive}
            onCloseTab={(i) => {
              setTabs((prev) => prev.filter((_, n) => n !== i));
              setActive(0);
            }}
            entries={[]}
            folders={[]}
            projects={[]}
            onRefetchIndex={() => {}}
            onRefetchFolders={() => {}}
            onProjectChanged={() => {}}
            onProjectGone={() => {}}
          />
        </QueryClientProvider>
      );
    }
    render(<Harness />);
    await waitFor(() =>
      expect(screen.getAllByLabelText("File content editor")).toHaveLength(2),
    );

    // Dirty it, then close it through the confirm.
    fireEvent.change(screen.getAllByLabelText("File content editor")[0], {
      target: { value: "alpha EDITED" },
    });
    expect(await screen.findByLabelText("Unsaved changes")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Close a\.md/ }));
    fireEvent.click(screen.getByRole("button", { name: /discard and close/i }));
    await waitFor(() => expect(screen.queryByTitle("p — a.md")).toBeNull());

    // Reopen the same file: clean tab, no marker, and closing it must not ask.
    fireEvent.click(screen.getByRole("button", { name: /reopen a\.md/ }));
    await waitFor(() =>
      expect(screen.getAllByLabelText("File content editor")).toHaveLength(2),
    );
    expect(screen.queryByLabelText("Unsaved changes")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /Close a\.md/ }));
    expect(screen.queryByText(/closing discards them/i)).toBeNull();
  });

  it("closes a clean tab without asking", async () => {
    const { onCloseTab } = renderTwoTabs();
    await waitFor(() =>
      expect(screen.getAllByLabelText("File content editor")).toHaveLength(2),
    );
    fireEvent.click(screen.getByRole("button", { name: /Close a\.md/ }));
    expect(onCloseTab).toHaveBeenCalledWith(0);
  });

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

  // Measurement rendering is covered by MeasurementView.test.tsx — that
  // component lives on the Context Manager subtab, not in an editor tab.
});
