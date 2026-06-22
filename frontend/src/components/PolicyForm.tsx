import type { ForcePushMode, Policy, PushGateMode } from "../lib/bindings";
import { cn } from "../lib/cn";
import { terminalInputClass } from "../app/contextLibraryShared";
import { Button } from "./ui/Button";

/**
 * Controlled, presentational editor for a {@link Policy}. Shared across all
 * three policy tiers (global / project / session) — the parent owns load/save/
 * dirty state and feeds `value` + `onChange`. Every `Policy` field is optional
 * (serde `default`), so we normalize to concrete UI values here.
 *
 * Scope note: a `Policy` carries no `tool_gate` — gated-Bash keywords live in
 * one global list (Settings → Tool Gate). The session tier preserves its frozen
 * snapshot keywords on save; this form never touches them.
 */
export function PolicyForm({
  value,
  onChange,
  disabled = false,
}: {
  value: Policy;
  onChange: (next: Policy) => void;
  disabled?: boolean;
}) {
  const patch = (p: Partial<Policy>) => onChange({ ...value, ...p });

  return (
    <div className="flex flex-col gap-6">
      <div className="grid grid-cols-1 gap-6 sm:grid-cols-2">
        <Field
          label="Push gate"
          hint="auto: pushes go through · ask: the pre-push hook blocks until set to auto"
        >
          <SegToggle<PushGateMode>
            value={value.push_gate ?? "auto"}
            disabled={disabled}
            onChange={(push_gate) => patch({ push_gate })}
            options={[
              { value: "auto", label: "Auto", tone: "good" },
              { value: "ask", label: "Ask", tone: "warn" },
            ]}
          />
        </Field>
        <Field
          label="Force push"
          hint="allowed: --force permitted (still subject to push gate) · blocked: denied"
        >
          <SegToggle<ForcePushMode>
            value={value.force_push ?? "allowed"}
            disabled={disabled}
            onChange={(force_push) => patch({ force_push })}
            options={[
              { value: "allowed", label: "Allowed", tone: "good" },
              { value: "blocked", label: "Blocked", tone: "danger" },
            ]}
          />
        </Field>
      </div>

      <Field
        label="Per-action approval"
        hint="Bash command prefixes that require request_approval on every invocation."
      >
        <StringList
          items={value.per_action_approval ?? []}
          disabled={disabled}
          placeholder="e.g. terraform apply"
          onChange={(per_action_approval) => patch({ per_action_approval })}
        />
      </Field>

      <Field label="Branch pattern" hint="Regex branch names must match. Empty = no constraint.">
        <input
          type="text"
          value={value.branch_pattern ?? ""}
          disabled={disabled}
          onChange={(e) => patch({ branch_pattern: e.target.value })}
          placeholder="(no constraint)"
          className={terminalInputClass}
        />
      </Field>
    </div>
  );
}

// ----------------------------------------------------------------------------

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <label className="block">
      <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
        {label}
      </span>
      {children}
      {hint && (
        <span className="mt-1 block font-code-sm text-code-sm text-on-surface-variant/70">
          {hint}
        </span>
      )}
    </label>
  );
}

type SegTone = "good" | "warn" | "danger";

function SegToggle<T extends string>({
  value,
  options,
  onChange,
  disabled,
}: {
  value: T;
  options: { value: T; label: string; tone: SegTone }[];
  onChange: (v: T) => void;
  disabled?: boolean;
}) {
  return (
    <div className="flex overflow-hidden rounded border border-outline-variant">
      {options.map((o) => {
        const active = value === o.value;
        const activeCls =
          o.tone === "good"
            ? "bg-success/15 text-success"
            : o.tone === "warn"
              ? "bg-primary/15 text-primary"
              : "bg-error/20 text-on-error-container";
        return (
          <button
            key={o.value}
            type="button"
            disabled={disabled}
            onClick={() => onChange(o.value)}
            className={cn(
              "px-3 py-1 font-label-caps text-label-caps transition-colors disabled:opacity-50",
              active
                ? activeCls
                : "bg-transparent text-on-surface-variant hover:text-on-surface",
            )}
          >
            {o.label}
          </button>
        );
      })}
    </div>
  );
}

function StringList({
  items,
  placeholder,
  onChange,
  disabled,
}: {
  items: string[];
  placeholder?: string;
  onChange: (next: string[]) => void;
  disabled?: boolean;
}) {
  const update = (i: number, v: string) =>
    onChange(items.map((it, idx) => (idx === i ? v : it)));
  const remove = (i: number) => onChange(items.filter((_, idx) => idx !== i));
  const add = () => onChange([...items, ""]);

  return (
    <div className="flex flex-col gap-2">
      {items.length === 0 && (
        <p className="font-code-sm text-code-sm text-on-surface-variant/70">
          None — nothing enforced for this list.
        </p>
      )}
      {items.map((it, i) => (
        <div key={i} className="flex items-center gap-2">
          <input
            type="text"
            value={it}
            disabled={disabled}
            placeholder={placeholder}
            onChange={(e) => update(i, e.target.value)}
            className={cn(terminalInputClass, "flex-1")}
          />
          <button
            type="button"
            disabled={disabled}
            onClick={() => remove(i)}
            aria-label="Remove entry"
            className="shrink-0 rounded border border-outline-variant bg-transparent px-2 py-1 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:bg-surface-container-high hover:text-on-surface disabled:opacity-50"
          >
            ✕
          </button>
        </div>
      ))}
      <div>
        <Button variant="ghost" size="sm" onClick={add} disabled={disabled}>
          + Add
        </Button>
      </div>
    </div>
  );
}
