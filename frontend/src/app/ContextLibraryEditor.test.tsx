import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { EditorArea } from "./ContextLibraryEditor";
import { invoke } from "@tauri-apps/api/core";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockInvoke = vi.mocked(invoke);

function renderEditor() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const tab = { kind: "file" as const, project: "p", filePath: "a.md" };
  return render(
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
        onProjectChanged={() => {}}
      />
    </QueryClientProvider>,
  );
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
});
