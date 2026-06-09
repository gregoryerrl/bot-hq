import { useState } from "react";
import { Button } from "./ui/Button";

/** Shape ChoicePrompt renders — a pending choice/approval row. Hand-defined
 *  (not a generated binding) because it's built frontend-side from a durable
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
  /** The option string currently mid-resolve for this choice, or undefined. */
  pendingOption: string | undefined;
  onResolve: (choiceId: string, picked: string) => void;
}

/**
 * One in-chat question: the preset options PLUS a mandatory "Other" free-text
 * field (#8). The field is ALWAYS present so the user can answer outside the
 * preset options — `resolve_choice` accepts arbitrary text as the picked value
 * (there is no options-membership check server-side), and the agent receives
 * whatever the user typed verbatim.
 */
export function ChoicePrompt({
  choice,
  pendingOption,
  onResolve,
}: ChoicePromptProps) {
  const [other, setOther] = useState("");
  const isPending = pendingOption !== undefined;
  const otherIsPending = isPending && !choice.options.includes(pendingOption!);

  const submitOther = () => {
    const text = other.trim();
    if (!text || isPending) return;
    onResolve(choice.choice_id, text);
    setOther("");
  };

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
              disabled={isPending}
              onClick={() => onResolve(choice.choice_id, opt)}
            >
              {pendingOption === opt ? `${opt} …` : opt}
            </Button>
          ))}
        </div>
      )}

      <div className="mt-2 flex items-center gap-1.5">
        <input
          type="text"
          value={other}
          disabled={isPending}
          onChange={(e) => setOther(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              submitOther();
            }
          }}
          placeholder="Other — type a custom answer…"
          className="min-w-0 flex-1 rounded border border-outline/40 bg-surface px-2 py-1 font-mono text-xs text-on-surface placeholder:text-on-surface-variant/70 focus:border-secondary focus:outline-none disabled:opacity-50"
        />
        <Button
          size="sm"
          variant="ghost"
          disabled={isPending || other.trim() === ""}
          onClick={submitOther}
        >
          {otherIsPending ? "Sending…" : "Send"}
        </Button>
      </div>
    </div>
  );
}
