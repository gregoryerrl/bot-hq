import { fireEvent, render, screen } from "@testing-library/react";
import { Button } from "./Button";

describe("Button", () => {
  // A typeless button inside a form submits it: the composer is a <form>, so
  // a Copy or Cancel there would send the draft (EYES, plan point 2).
  it("is type=button unless asked to submit", () => {
    const onSubmit = vi.fn((e: { preventDefault: () => void }) => e.preventDefault());
    render(
      <form onSubmit={onSubmit}>
        <Button>Copy</Button>
        <Button type="submit">Send</Button>
      </form>,
    );
    expect(screen.getByRole("button", { name: "Copy" })).toHaveAttribute("type", "button");
    fireEvent.click(screen.getByRole("button", { name: "Copy" }));
    expect(onSubmit).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Send" })).toHaveAttribute("type", "submit");
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  it("is a mono label in every boxed size, and every variant keeps a 1px border", () => {
    for (const variant of ["primary", "secondary", "ghost", "danger"] as const) {
      for (const size of ["sm", "md"] as const) {
        render(
          <Button variant={variant} size={size}>
            {`${variant}-${size}`}
          </Button>,
        );
        const b = screen.getByRole("button", { name: `${variant}-${size}` });
        expect(b).toHaveClass("font-code-sm", "text-code-sm", "border", "rounded");
      }
    }
  });

  it("draws an icon button as a 28px square and keeps its label", () => {
    render(
      <Button size="icon" variant="ghost" aria-label="Close">
        <svg />
      </Button>,
    );
    const b = screen.getByRole("button", { name: "Close" });
    expect(b).toHaveClass("size-7");
    expect(b).not.toHaveClass("px-2");
  });

  // `cn` is plain clsx: a text colour passed in className lands beside the
  // variant's and the stylesheet's NAME order decides (text-error lost to
  // text-on-surface). A tone REPLACES the variant's colour instead.
  it("takes a tone in place of the variant's own label colour", () => {
    render(
      <>
        <Button tone="error">Delete</Button>
        <Button variant="link" tone="inherit">
          dismiss
        </Button>
        <Button>Plain</Button>
      </>,
    );
    const del = screen.getByRole("button", { name: "Delete" });
    expect(del).toHaveClass("text-error");
    expect(del).not.toHaveClass("text-on-surface");
    const dismiss = screen.getByRole("button", { name: "dismiss" });
    expect(dismiss).toHaveClass("text-inherit");
    expect(dismiss).not.toHaveClass("text-primary");
    expect(screen.getByRole("button", { name: "Plain" })).toHaveClass("text-on-surface");
  });

  it("lets a link-styled action take its text's size", () => {
    render(<Button variant="link">open the tray</Button>);
    const b = screen.getByRole("button", { name: "open the tray" });
    expect(b).toHaveClass("inline", "underline");
    expect(b).not.toHaveClass("font-code-sm");
    expect(b).not.toHaveClass("px-3");
  });
});
