import { cn } from "../lib/cn";

/**
 * Dismissible inline error banner in the Industrial-Terminal error styling.
 * `label` is the bold lead (e.g. "Send failed:"); `className` overrides only the
 * outer margins per call site.
 */
export function ErrorBanner({
  label,
  message,
  onDismiss,
  className,
}: {
  label: string;
  message: string;
  onDismiss: () => void;
  className?: string;
}) {
  return (
    <div
      role="alert"
      className={cn(
        "rounded border border-error/40 bg-error-container/30 px-3 py-1.5 text-xs text-on-error-container",
        className,
      )}
    >
      <span className="font-semibold">{label}</span> {message}
      <button
        type="button"
        className="ml-2 underline hover:text-error"
        onClick={onDismiss}
      >
        dismiss
      </button>
    </div>
  );
}
