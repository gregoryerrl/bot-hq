import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { Markdown } from "./Markdown";

describe("Markdown", () => {
  it("renders headings, links, and lists from markdown source", () => {
    render(
      <Markdown>
        {"# Title\n\nSee [docs](https://example.com)\n\n- one\n- two"}
      </Markdown>,
    );
    expect(screen.getByRole("heading", { name: "Title" })).toBeInTheDocument();
    const link = screen.getByRole("link", { name: "docs" });
    expect(link).toHaveAttribute("href", "https://example.com");
    expect(link).toHaveAttribute("target", "_blank");
    expect(screen.getByText("one")).toBeInTheDocument();
    expect(screen.getByText("two")).toBeInTheDocument();
  });

  it("renders inline code without crashing", () => {
    render(<Markdown>{"use `cargo test` to run"}</Markdown>);
    expect(screen.getByText("cargo test")).toBeInTheDocument();
  });
});
