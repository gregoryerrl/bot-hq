import type { ReactNode } from "react";
import { cn } from "../lib/cn";
import type { ParticipantView } from "../lib/participants";

/**
 * One participant in the session header's roster, as ONE unbroken unit (the
 * visual pass, S4): its hue dot, its `ROLE · Model` label — which opens the
 * system prompt it spawned with — and its badges (`children`: the spawn knob,
 * an account tag, the health dot, the context meter, the backlog). The roster
 * row wraps BETWEEN chips, never inside one: the old inline run broke a
 * participant's label from its meter at the window's edge.
 */
export function ParticipantChip({
  label,
  hueClass,
  enabled,
  title,
  onOpenPrompt,
  children,
}: {
  label: string;
  /** The participant's hue as a text class (`authorColorClass`), for the
   *  label and, through `bg-current`, the dot — the chat's byline colours. */
  hueClass: string;
  enabled: boolean;
  title: string;
  onOpenPrompt: () => void;
  children?: ReactNode;
}) {
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center gap-1.5 whitespace-nowrap rounded border border-outline-variant px-1.5 py-0.5",
        !enabled && "opacity-50",
      )}
      title={title}
    >
      <span aria-hidden className={cn("size-2 shrink-0 rounded-full bg-current", hueClass)} />
      {/* rc3 P1: the label is the way in to what this participant was
          actually told. Attribution is structural — the chip you click IS
          the participant, so a prompt can't be shown under the wrong
          heading. */}
      <button
        type="button"
        onClick={onOpenPrompt}
        title="View the composed system prompt this participant spawned with"
        className={cn(
          "font-label-caps text-label-caps underline decoration-dotted underline-offset-2 transition-colors hover:decoration-solid",
          hueClass,
        )}
      >
        {label}
      </button>
      {children}
    </span>
  );
}

/**
 * The account a participant bills, as a SHORT tag for a roster whose
 * participants bill different accounts: the config dir's name (`default` for
 * `~/.claude`, `acct-2` for `~/.claude-acct-2`), else the email's local part.
 * `null` when the spawn recorded nothing (pre-0090, or a gateway participant).
 */
export function accountTagOf(
  p: Pick<ParticipantView, "account_dir_at_spawn" | "account_email_at_spawn">,
): string | null {
  const dir = p.account_dir_at_spawn?.trim().replace(/[\\/]+$/, "") ?? null;
  if (dir === "") return "default";
  if (dir) {
    const last = dir.split(/[\\/]/).pop() ?? dir;
    if (last === ".claude") return "default";
    return last.replace(/^\.?claude-/, "") || last;
  }
  const email = p.account_email_at_spawn?.trim();
  return email ? email.split("@")[0] : null;
}
