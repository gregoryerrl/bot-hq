import { fireEvent, render, screen } from "@testing-library/react";
import { ParticipantChip, accountTagOf } from "./ParticipantChip";

describe("ParticipantChip (the visual pass, S4)", () => {
  it("is one unbroken unit: hue dot, the label that opens the prompt, the badges", () => {
    const onOpenPrompt = vi.fn();
    render(
      <ParticipantChip label="HANDS · 2nd Opus Latest" hueClass="text-author-orange" enabled title="Participant 1" onOpenPrompt={onOpenPrompt}>
        <span>ctx</span>
      </ParticipantChip>,
    );
    const chip = screen.getByTitle("Participant 1");
    expect(chip).toHaveClass("whitespace-nowrap", "inline-flex", "shrink-0");
    expect(chip).toHaveTextContent("HANDS · 2nd Opus Latest");
    expect(chip).toHaveTextContent("ctx");
    expect(chip.querySelector('[aria-hidden="true"]')).toHaveClass("bg-current", "text-author-orange");
    fireEvent.click(screen.getByRole("button", { name: "HANDS · 2nd Opus Latest" }));
    expect(onOpenPrompt).toHaveBeenCalledTimes(1);
  });

  it("dims a participant that is not running", () => {
    render(<ParticipantChip label="EYES" hueClass="text-author-violet" enabled={false} title="off" onOpenPrompt={() => {}} />);
    expect(screen.getByTitle("off")).toHaveClass("opacity-50");
  });
});

describe("accountTagOf", () => {
  it("names the config dir briefly, else the email's local part, else nothing", () => {
    expect(accountTagOf({ account_dir_at_spawn: "", account_email_at_spawn: "one@example.com" })).toBe("default");
    expect(accountTagOf({ account_dir_at_spawn: "/Users/me/.claude", account_email_at_spawn: null })).toBe("default");
    expect(accountTagOf({ account_dir_at_spawn: "/Users/me/.claude-acct-2/", account_email_at_spawn: null })).toBe("acct-2");
    expect(accountTagOf({ account_dir_at_spawn: "D:\\\\claude\\\\work", account_email_at_spawn: null })).toBe("work");
    expect(accountTagOf({ account_dir_at_spawn: null, account_email_at_spawn: "two@example.com" })).toBe("two");
    expect(accountTagOf({ account_dir_at_spawn: null, account_email_at_spawn: null })).toBeNull();
  });
});
