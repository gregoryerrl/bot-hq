import { render, screen } from "@testing-library/react";
import { Chip, chipTone, SaveStateChip, type ChipTone } from "./Chip";

const TONES: ChipTone[] = ["neutral", "primary", "secondary", "tertiary", "success", "warning", "error"];

describe("Chip", () => {
  it("is a label-caps chip with the house radius, in every tone", () => {
    for (const tone of TONES) {
      render(<Chip tone={tone}>{tone}</Chip>);
      const chip = screen.getByText(tone);
      expect(chip).toHaveClass("rounded", "border", "font-label-caps", "text-label-caps", "whitespace-nowrap");
      for (const c of chipTone(tone).split(" ")) expect(chip).toHaveClass(c);
    }
  });

  it("tints a coloured tone at 15% with a 40% outline and the full colour for the text", () => {
    expect(chipTone("warning")).toBe("border-warning/40 bg-warning/15 text-warning");
    expect(chipTone("tertiary")).toBe("border-tertiary/40 bg-tertiary/15 text-tertiary");
  });

  it("stays one unbroken label unless asked to wrap", () => {
    render(<Chip tone="warning">Unsaved</Chip>);
    expect(screen.getByText("Unsaved")).toHaveClass("whitespace-nowrap", "shrink-0");
    render(
      <Chip tone="primary" wrap>
        Needs you · halted — your move
      </Chip>,
    );
    const long = screen.getByText("Needs you · halted — your move");
    expect(long).toHaveClass("whitespace-normal", "break-words", "max-w-full");
    expect(long).not.toHaveClass("whitespace-nowrap");
  });

  it("keeps the caller's classes and attributes", () => {
    render(
      <Chip tone="primary" size="sm" className="ml-1" title="2 waiting">
        2
      </Chip>,
    );
    const chip = screen.getByTitle("2 waiting");
    expect(chip).toHaveClass("ml-1", "px-1");
  });
});

describe("SaveStateChip", () => {
  it("says Unsaved while dirty, Saved with a check once written, and nothing otherwise", () => {
    const { rerender, container } = render(<SaveStateChip dirty saved={false} />);
    expect(screen.getByText("Unsaved")).toHaveClass("text-warning");

    rerender(<SaveStateChip dirty={false} saved />);
    const saved = screen.getByText("Saved");
    expect(saved).toHaveClass("text-success");
    expect(saved.querySelector("svg")).not.toBeNull();

    // Dirty wins: an edit after a save is unsaved again.
    rerender(<SaveStateChip dirty saved />);
    expect(screen.getByText("Unsaved")).toBeInTheDocument();

    rerender(<SaveStateChip dirty={false} saved={false} />);
    expect(container).toBeEmptyDOMElement();
  });
});
