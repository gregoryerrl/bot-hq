import { fireEvent, render, screen } from "@testing-library/react";
import { Button } from "./Button";
import { Dialog, scrimClass } from "./Dialog";

function renderDialog(onClose = vi.fn()) {
  render(
    <>
      <button type="button">trigger</button>
      <Dialog
        open
        onClose={onClose}
        title="Discard changes?"
        footer={
          <>
            <Button variant="ghost">Cancel</Button>
            <Button variant="danger">Discard changes</Button>
          </>
        }
      >
        <p>Your edits to this role are not saved.</p>
      </Dialog>
    </>,
  );
  return onClose;
}

describe("Dialog", () => {
  it("is a labelled modal with its body and its actions", () => {
    renderDialog();
    const dialog = screen.getByRole("dialog", { name: "Discard changes?" });
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(dialog).toHaveTextContent("Your edits to this role are not saved.");
    expect(screen.getByRole("button", { name: "Discard changes" })).toBeInTheDocument();
  });

  it("closes from Escape, the scrim and the close button", () => {
    const onClose = renderDialog();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
    const scrim = screen.getByTestId("dialog-scrim");
    expect(scrim).toHaveClass(...scrimClass.split(" "));
    fireEvent.click(scrim);
    expect(onClose).toHaveBeenCalledTimes(2);
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(onClose).toHaveBeenCalledTimes(3);
  });

  it("keeps the scrim a sibling of the frame, never its parent", () => {
    renderDialog();
    const dialog = screen.getByRole("dialog");
    expect(dialog.parentElement?.className ?? "").not.toContain("inset-0");
  });

  it("moves focus into the frame on open", () => {
    renderDialog();
    expect(screen.getByRole("dialog").contains(document.activeElement)).toBe(true);
  });

  it("renders nothing while closed", () => {
    render(
      <Dialog open={false} onClose={() => {}} title="Hidden">
        <p>body</p>
      </Dialog>,
    );
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
