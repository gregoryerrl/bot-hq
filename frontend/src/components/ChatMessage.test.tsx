import { render, screen } from "@testing-library/react";
import type { AgentMessage } from "../lib/bindings";
import { ChatMessage } from "./ChatMessage";

const msg = (author: string, kind: string, content: string): AgentMessage => ({
  id: 1,
  session_id: "s1",
  author,
  kind,
  content,
  created_at: "2026-10-19T05:00:00Z",
});

describe("ChatMessage (the visual pass, S3)", () => {
  it("renders a restarting host prompt as a system row without a participant byline", () => {
    const { container } = render(
      <ChatMessage message={msg("system", "system_prompt", "Record any remaining learnings.")} authorLabel="System" />,
    );
    expect(screen.getByText("Record any remaining learnings.")).toBeInTheDocument();
    expect(container.querySelector("article")).toBeNull();
    expect(container.querySelector("header")).toBeNull();
  });

  it("sets the user's own message on the right, in their blue", () => {
    const { container } = render(<ChatMessage message={msg("user", "text", "proceed please")} authorLabel="You" />);
    const article = container.querySelector("article");
    expect(article).toHaveClass("items-end");
    const bubble = screen.getByText("proceed please").closest("div.rounded");
    expect(bubble).toHaveClass("border-tertiary/30", "bg-tertiary/10");
  });

  it("leaves an agent's message on the left, unboxed", () => {
    const { container } = render(<ChatMessage message={msg("hands", "text", "Committed.")} authorLabel="HANDS" />);
    expect(container.querySelector("article")).not.toHaveClass("items-end");
    expect(screen.getByText("Committed.").closest("div.rounded")).toBeNull();
  });

  it("shows a notice's warning once: the icon, not the notice's own glyph as well", () => {
    render(<ChatMessage message={msg("user", "system_notice", "⚠ hands's context was compacted")} />);
    const notice = screen.getByText("hands's context was compacted");
    expect(notice.textContent).not.toContain("⚠");
    expect(notice.querySelector("svg")).not.toBeNull();
    expect(notice).not.toHaveClass("italic");
    // The emoji form of the glyph goes too.
    render(<ChatMessage message={msg("user", "system_notice", "⚠️ [bot-hq] hands hit a provider limit")} />);
    expect(screen.getByText("[bot-hq] hands hit a provider limit").textContent).not.toContain("⚠");
  });
});
