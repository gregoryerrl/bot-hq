import { cn } from "../lib/cn";

/**
 * Dismissible inline error banner in the Industrial-Terminal error styling.
 * `label` is the bold lead (e.g. "Send failed:"); `className` overrides only the
 * outer margins per call site.
 *
 * `edge` renders the full-width variant that sits flush against a container
 * edge (0 radius, a bottom rule, mono) — DESIGN.md's banner shape — which the
 * SessionView used to hand-roll three times (round 11). `dismissLabel` names
 * the one action; the respawn banner's action is a retry, not a dismiss.
 */
export function ErrorBanner({
  label,
  message,
  onDismiss,
  className,
  edge = false,
  dismissLabel = "dismiss",
}: {
  label: string;
  message: string;
  onDismiss: () => void;
  className?: string;
  edge?: boolean;
  dismissLabel?: string;
}) {
  return (
    <div
      role="alert"
      className={cn(
        edge
          ? "border-b border-outline-variant bg-error-container/30 px-4 py-2 font-code-sm text-code-sm text-on-error-container"
          : "rounded border border-error/40 bg-error-container/30 px-3 py-1.5 text-xs text-on-error-container",
        className,
      )}
    >
      <span className="font-semibold">{label}</span> {message}
      <button
        type="button"
        className="ml-2 underline hover:text-error"
        onClick={onDismiss}
      >
        {dismissLabel}
      </button>
    </div>
  );
}
