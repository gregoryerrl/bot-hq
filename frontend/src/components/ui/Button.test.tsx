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

  it("lets a link-styled action take its text's size", () => {
    render(<Button variant="link">open the tray</Button>);
    const b = screen.getByRole("button", { name: "open the tray" });
    expect(b).toHaveClass("inline", "underline");
    expect(b).not.toHaveClass("font-code-sm");
    expect(b).not.toHaveClass("px-3");
  });
});
