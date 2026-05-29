import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { ContextMenu, ActionModal } from "./ContextLibraryContextMenu";

describe("ContextMenu", () => {
  it("invokes the selected item and then closes", () => {
    const onSelect = vi.fn();
    const onClose = vi.fn();
    render(
      <ContextMenu
        x={10}
        y={10}
        onClose={onClose}
        items={[{ label: "New file", onSelect }]}
      />,
    );
    fireEvent.click(screen.getByRole("menuitem", { name: "New file" }));
    expect(onSelect).toHaveBeenCalled();
    expect(onClose).toHaveBeenCalled();
  });

  it("closes on Escape", () => {
    const onClose = vi.fn();
    render(
      <ContextMenu
        x={0}
        y={0}
        onClose={onClose}
        items={[{ label: "X", onSelect: () => {} }]}
      />,
    );
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
  });
});

describe("ActionModal", () => {
  it("prompts for a name and confirms with the trimmed value", () => {
    const onConfirm = vi.fn();
    render(
      <ActionModal
        title="New file"
        inputLabel="File name"
        confirmLabel="Create"
        onConfirm={onConfirm}
        onClose={() => {}}
      />,
    );
    fireEvent.change(screen.getByLabelText("File name"), {
      target: { value: "  notes.md  " },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    expect(onConfirm).toHaveBeenCalledWith("notes.md");
  });

  it("confirm-only (delete) calls onConfirm with an empty string", () => {
    const onConfirm = vi.fn();
    render(
      <ActionModal
        title="Delete folder"
        message='Delete "x"?'
        confirmLabel="Delete"
        danger
        onConfirm={onConfirm}
        onClose={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    expect(onConfirm).toHaveBeenCalledWith("");
  });
});
