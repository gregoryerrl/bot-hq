import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { useTauriQuery, useTauriMutation } from "../hooks/useInvoke";
import { Button } from "../components/ui/Button";

/**
 * The one-time starter offers (1.0.1) — same contract as the Roles tab's
 * example-pair card: the card renders ONLY while the app setting is the
 * literal `pending` (absent = no offer, so configured installs never see it);
 * Install writes the starter file — never over an existing one — and Decline
 * stamps it away forever. Two independent keys so gates and policy can be
 * adopted separately.
 */
const OFFERS = {
  gates: {
    settingKey: "gate_preset_offer",
    resolveCommand: "resolve_gate_preset_offer",
    title: "Want a starting point?",
    body:
      "Install the basic safety gates: destructive commands (rm -r, sudo, " +
      "disk writers, git reset --hard, git clean -f) park for your " +
      "Approve/Reject before they run. A gated command pauses the session " +
      "until you answer. Note the sudo gate also catches package installs " +
      "(sudo dnf / sudo apt) — if that's too chatty for your machine, edit " +
      "or remove any keyword below.",
    install: "Install basic gates",
  },
  policy: {
    settingKey: "policy_preset_offer",
    resolveCommand: "resolve_policy_preset_offer",
    title: "Want a starting point?",
    body:
      "Install the basic policy: every git push asks for your approval (the " +
      "session pauses until you answer) and force-push is refused. It applies " +
      "to every project that doesn't set its own — and a project file cannot " +
      "relax the push gate back to auto; use the session's gear toggle for " +
      "that. The commit-blocking word list ships empty — the file's comments " +
      "show how to add your own.",
    install: "Install basic policy",
  },
} as const;

export function PresetOfferCard({ kind }: { kind: keyof typeof OFFERS }) {
  const offer = OFFERS[kind];
  // Literal-'pending' rule (RolesPanel precedent): null / absent / any other
  // value renders nothing.
  const { data: flag = null, refetch } = useTauriQuery<string | null>(
    "get_app_setting",
    { key: offer.settingKey },
  );
  const resolve = useTauriMutation<null, { install: boolean }>(
    offer.resolveCommand,
  );
  if (flag !== "pending") return null;
  const answer = (install: boolean) =>
    resolve.mutate({ install }, { onSuccess: () => void refetch() });
  return (
    <div className="mb-6 max-w-prose rounded-md border border-outline-variant bg-surface-container-low p-3">
      <p className="font-body-md text-body-md text-on-surface">
        <span className="font-medium">{offer.title}</span> {offer.body}
      </p>
      <div className="mt-2 flex gap-2">
        <Button
          variant="primary"
          onClick={() => answer(true)}
          disabled={resolve.isPending}
        >
          {offer.install}
        </Button>
        <Button
          variant="ghost"
          onClick={() => answer(false)}
          disabled={resolve.isPending}
        >
          No thanks
        </Button>
      </div>
    </div>
  );
}

const BANNER_DISMISS_KEY = "preset_offer_banner_dismissed";

/**
 * The Dashboard's entry point to the offers. Keyed off the offer FLAGS, never
 * off an empty session list — the installs this feature targets already have
 * sessions, so an empty-state-only surface would miss exactly them (F5).
 * Dismiss is UI-local (localStorage): it hides the banner on this machine but
 * never stamps the keys — only the Settings cards resolve an offer.
 */
export function PresetOfferBanner() {
  const navigate = useNavigate();
  const [dismissed, setDismissed] = useState(
    () => localStorage.getItem(BANNER_DISMISS_KEY) === "1",
  );
  const { data: gates = null } = useTauriQuery<string | null>(
    "get_app_setting",
    { key: "gate_preset_offer" },
  );
  const { data: policy = null } = useTauriQuery<string | null>(
    "get_app_setting",
    { key: "policy_preset_offer" },
  );
  const pending = gates === "pending" || policy === "pending";
  if (dismissed || !pending) return null;
  const dismiss = () => {
    localStorage.setItem(BANNER_DISMISS_KEY, "1");
    setDismissed(true);
  };
  return (
    <div className="mb-4 flex items-center justify-between gap-3 rounded-md border border-outline-variant bg-surface-container-low px-3 py-2">
      <p className="font-body-md text-body-md text-on-surface">
        Starter safety defaults are available — basic command gates and a base
        policy you can adopt with one click, then edit any time.
      </p>
      <div className="flex shrink-0 items-center gap-2">
        <Button
          variant="primary"
          onClick={() =>
            navigate(
              `/settings?tab=${gates === "pending" ? "toolgate" : "policy"}`,
            )
          }
        >
          Review offers
        </Button>
        <Button variant="ghost" onClick={dismiss} aria-label="Dismiss">
          ✕
        </Button>
      </div>
    </div>
  );
}
