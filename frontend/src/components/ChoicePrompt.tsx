import { useState } from "react";
import { Button } from "./ui/Button";
import { cn } from "../lib/cn";

/** Shape ChoicePrompt renders — a pending question row. Hand-defined (not a
 *  generated binding) because it's built frontend-side from a durable
 *  SessionTrayView, not returned by any Tauri command. */
export interface ChoicePromptChoice {
  choice_id: string;
  session_id: string;
  agent: string;
  question: string;
  options: string[];
}

interface ChoicePromptProps {
  choice: ChoicePromptChoice;
  /** The pick currently staged for this question, if any. Highlights the
   *  matching option; a staged custom answer seeds nothing here (the staged
   *  chip below the card shows it verbatim). */
  stagedOption?: string | undefined;
  /** Stage a preset option. */
  onPick: (choiceId: string, picked: string) => void;
  /** The Other box changed. Fires per keystroke with the current text; empty
   *  text means the custom answer is withdrawn. */
  onOther: (choiceId: string, text: string) => void;
}

/**
 * One tray question: the preset options PLUS a mandatory "Other" free-text
 * field (#8), so the user can answer outside the presets — the picked value is
 * arbitrary text server-side and the agent receives it verbatim.
 *
 * **Nothing here sends (rc3 D35).** The user, after the second regression:
 * *"I thought I was clear on this that answers will be sent in one batch.
 * REMOVE THE SEND BUTTON ON QUESTIONS. I type on the 'other:' box on question,
 * then click send from the input box, all answers including my message will
 * get sent."* An option click stages; typing in Other stages; the composer's
 * Send delivers everything as one response. This component had a per-card Send
 * twice — first for resolve-on-click, then for the Other box — and both were
 * a second answer path racing the batch.
 */
export function ChoicePrompt({
  choice,
  stagedOption,
  onPick,
  onOther,
}: ChoicePromptProps) {
  const [other, setOther] = useState("");

  return (
    <div className="rounded border border-secondary/40 bg-secondary/5 p-3">
      <div className="mb-2 font-body-md text-body-md text-on-surface">
        {choice.question}
      </div>

      {choice.options.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {choice.options.map((opt) => (
            <Button
              key={opt}
              size="sm"
              variant="secondary"
              className={cn(
                stagedOption === opt && "ring-1 ring-primary text-primary",
              )}
              onClick={() => onPick(choice.choice_id, opt)}
            >
              {opt}
            </Button>
          ))}
        </div>
      )}

      <div className="mt-2">
        <input
          type="text"
          value={other}
          onChange={(e) => {
            setOther(e.target.value);
            onOther(choice.choice_id, e.target.value);
          }}
          placeholder="Other — type a custom answer; it sends with your message…"
          className="w-full rounded border border-outline/40 bg-surface px-2 py-1 font-mono text-xs text-on-surface placeholder:text-on-surface-variant/70 focus:border-secondary focus:outline-none"
        />
      </div>
    </div>
  );
}
