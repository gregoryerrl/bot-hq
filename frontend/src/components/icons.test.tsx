import { render } from "@testing-library/react";
import type { ComponentType } from "react";
import * as icons from "./icons";
import type { IconProps } from "./icons";

// One icon set (the 2026-10-19 visual pass, S1d): every export is drawn on the
// one base, so no icon can drift to its own stroke, size mechanism or a11y.
const ALL: [string, ComponentType<IconProps>][] = Object.entries(icons).filter(
  ([name, value]) => name.endsWith("Icon") && typeof value === "function",
);

function svgOf(Icon: ComponentType<IconProps>, props: IconProps = {}) {
  const { container } = render(<Icon {...props} />);
  const svg = container.querySelector("svg");
  if (!svg) throw new Error("no svg rendered");
  return svg;
}

describe("icons", () => {
  it("exports the whole set, including the glyph replacements", () => {
    const names = ALL.map(([n]) => n);
    for (const n of [
      "CloseIcon",
      "WarnIcon",
      "ChevronRightIcon",
      "ChevronDownIcon",
      "CheckIcon",
      "CrossIcon",
      "PauseIcon",
      "ClockIcon",
      "BranchIcon",
      "EditIcon",
      "ArrowLeftIcon",
      "ArrowRightIcon",
      "SkipIcon",
      "DotIcon",
    ]) {
      expect(names).toContain(n);
    }
  });

  it.each(ALL)("%s is an outline at stroke 1.75, hidden from assistive tech, 16 px by class", (_, Icon) => {
    const svg = svgOf(Icon);
    expect(svg).toHaveAttribute("aria-hidden", "true");
    expect(svg).toHaveAttribute("stroke-width", "1.75");
    expect(svg).toHaveAttribute("stroke", "currentColor");
    expect(svg).toHaveAttribute("fill", "none");
    expect(svg).toHaveAttribute("viewBox", "0 0 24 24");
    expect(svg).toHaveClass("size-4");
    // Sized by class only: a width/height attribute would beat nothing and
    // fight the class at every call site.
    expect(svg).not.toHaveAttribute("width");
    expect(svg).not.toHaveAttribute("height");
  });

  it("drops the default size when the caller sizes the icon", () => {
    // `cn` is plain clsx, so `size-4 size-3` would resolve by stylesheet order
    // (`.size-4` sorts after `.size-3`) and the caller's size would lose.
    for (const cls of ["size-3", "md:size-5", "!size-2.5", "w-3 h-3", "group-hover:h-5"]) {
      const svg = svgOf(icons.CheckIcon, { className: cls });
      expect(svg).not.toHaveClass("size-4");
      for (const c of cls.split(" ")) expect(svg).toHaveClass(c);
    }
    // A class that is not a size keeps the default beside it.
    const svg = svgOf(icons.CheckIcon, { className: "mr-1 text-success" });
    expect(svg).toHaveClass("size-4", "mr-1", "text-success");
  });

  it("passes other attributes through", () => {
    const svg = svgOf(icons.WarnIcon, { "data-testid": "w", style: { opacity: 0.5 } } as IconProps);
    expect(svg).toHaveAttribute("data-testid", "w");
    expect(svg.style.opacity).toBe("0.5");
  });
});
