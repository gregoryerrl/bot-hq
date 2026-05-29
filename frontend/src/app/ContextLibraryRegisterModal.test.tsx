import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { RegisterProjectModal } from "./ContextLibraryRegisterModal";
import { invoke } from "@tauri-apps/api/core";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockInvoke = vi.mocked(invoke);

describe("RegisterProjectModal", () => {
  beforeEach(() => mockInvoke.mockReset());

  it("registers a folder as a project (name defaults to basename) and rescans", async () => {
    mockInvoke.mockResolvedValue(undefined);
    const onRegistered = vi.fn();
    const onClose = vi.fn();
    render(
      <RegisterProjectModal
        open
        onClose={onClose}
        onRegistered={onRegistered}
      />,
    );

    fireEvent.change(
      screen.getByPlaceholderText("/Users/you/Projects/my-project"),
      { target: { value: "/Users/me/Projects/widget" } },
    );
    fireEvent.click(screen.getByRole("button", { name: /^register$/i }));

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("cl_register_project", {
        name: "widget",
        displayName: "widget",
        workingRepoPath: null,
        clPath: "/Users/me/Projects/widget",
        description: null,
      }),
    );
    expect(mockInvoke).toHaveBeenCalledWith("cl_rescan", { project: "widget" });
    expect(onRegistered).toHaveBeenCalled();
    expect(onClose).toHaveBeenCalled();
  });

  it("renders nothing when closed", () => {
    const { container } = render(
      <RegisterProjectModal
        open={false}
        onClose={() => {}}
        onRegistered={() => {}}
      />,
    );
    expect(container).toBeEmptyDOMElement();
  });
});
