import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { FolderView } from "./ContextLibraryFolderView";
import { invoke } from "@tauri-apps/api/core";
import type { ClFolderView } from "../lib/bindings";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockInvoke = vi.mocked(invoke);

const folder = (
  project_id: string,
  folder_path: string,
  description: string,
): ClFolderView => ({
  id: 1,
  project_id,
  folder_path,
  description,
  tags: null,
  created_at: "",
  updated_at: "",
});

describe("FolderView", () => {
  beforeEach(() => mockInvoke.mockReset());

  it("edits and saves a folder description via cl_set_folder_description", async () => {
    mockInvoke.mockResolvedValue(undefined);
    const onSaved = vi.fn();
    render(
      <FolderView
        tab={{ kind: "folder", project: "p", folderPath: "archive" }}
        folders={[folder("p", "archive", "old stuff")]}
        project={null}
        onSaved={onSaved}
        onProjectChanged={() => {}}
      />,
    );

    const desc = screen.getByPlaceholderText(/what this folder holds/i);
    expect(desc).toHaveValue("old stuff");
    expect(screen.getByRole("button", { name: /save folder/i })).toBeDisabled();

    fireEvent.change(desc, { target: { value: "archived audits" } });
    const save = screen.getByRole("button", { name: /save folder/i });
    expect(save).toBeEnabled();

    fireEvent.click(save);
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("cl_set_folder_description", {
        project: "p",
        folderPath: "archive",
        description: "archived audits",
        tags: null,
      }),
    );
    expect(onSaved).toHaveBeenCalled();
  });

  it("labels the project-root folder with the project name", () => {
    render(
      <FolderView
        tab={{ kind: "folder", project: "bot-hq", folderPath: "" }}
        folders={[]}
        project={null}
        onSaved={() => {}}
        onProjectChanged={() => {}}
      />,
    );
    expect(screen.getByText("bot-hq")).toBeInTheDocument();
    expect(screen.getByText("project root")).toBeInTheDocument();
  });

  it("shows the project registration section on a project root and unregisters", async () => {
    mockInvoke.mockResolvedValue(undefined);
    const onProjectChanged = vi.fn();
    render(
      <FolderView
        tab={{ kind: "folder", project: "bot-hq", folderPath: "" }}
        folders={[]}
        project={{
          name: "bot-hq",
          display_name: "bot-hq",
          working_repo_path: "/Users/me/Projects/bot-hq",
          description: null,
          cl_path: "/Users/me/.bot-hq/projects/bot-hq",
        }}
        onSaved={() => {}}
        onProjectChanged={onProjectChanged}
      />,
    );

    // CL path + working repo surfaced
    expect(
      screen.getByText("/Users/me/.bot-hq/projects/bot-hq"),
    ).toBeInTheDocument();
    expect(screen.getByDisplayValue("/Users/me/Projects/bot-hq")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /unregister project/i }));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("cl_unregister_project", {
        name: "bot-hq",
      }),
    );
    expect(onProjectChanged).toHaveBeenCalled();
  });
});
