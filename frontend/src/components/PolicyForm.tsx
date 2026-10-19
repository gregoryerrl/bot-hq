import type { ForcePushMode, Policy, PushGateMode } from "../lib/bindings";
import { cn } from "../lib/cn";
import { Button } from "./ui/Button";
import { SegToggle } from "./ui/SegToggle";
import { useListEditor } from "../hooks/useListEditor";
import { CloseIcon } from "./icons";
import { fieldClass } from "./ui/field";

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
          hint="auto: pushes go through · ask: each git push waits for your Approve/Reject (the pre-push hook parks a gate)"
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
        hint="Bash command prefixes the agent is told to request_approval for on every invocation — described to the agent, not mechanically enforced (the Tool Gate is the enforced list)."
      >
        <StringList
          items={value.per_action_approval ?? []}
          disabled={disabled}
          placeholder="e.g. terraform apply"
          onChange={(per_action_approval) => patch({ per_action_approval })}
        />
      </Field>

      <Field
        label="Production reads"
        hint="Commands that read production data. A command that runs one of these parks for your approval, for every participant, after the reviewer reads it. Matched where the command runs, not in quoted text; the last word may be part of a host."
      >
        <StringList
          items={value.production_reads ?? []}
          disabled={disabled}
          placeholder="e.g. gcloud logging read, psql -h my-prod-host"
          onChange={(production_reads) => patch({ production_reads })}
        />
      </Field>

      <Field
        label="Staging reads"
        hint="The same for staging data: parked for your approval, after the reviewer reads it."
      >
        <StringList
          items={value.staging_reads ?? []}
          disabled={disabled}
          placeholder="e.g. psql -h my-staging-host"
          onChange={(staging_reads) => patch({ staging_reads })}
        />
      </Field>

      <Field
        label="Branch pattern"
        hint="Regex the agent is told branch names must match — described to the agent, not mechanically enforced. Empty = no constraint."
      >
        <input
          type="text"
          value={value.branch_pattern ?? ""}
          disabled={disabled}
          onChange={(e) => patch({ branch_pattern: e.target.value })}
          placeholder="(no constraint)"
          className={fieldClass}
        />
      </Field>

      <Field
        label="Forbidden words in commits"
        hint="Words no commit message may carry: the commit-msg hook blocks them, the post-commit verifier logs a slip, and check_commit_message pre-checks the agent's message. They also screen the lines a commit adds unless the file-content list below is set. Whole-word, case-insensitive. A non-empty project list replaces the global one."
      >
        <StringList
          items={value.forbidden_in_commits ?? []}
          disabled={disabled}
          placeholder="e.g. a trailer no commit may carry"
          onChange={(forbidden_in_commits) => patch({ forbidden_in_commits })}
        />
      </Field>

      <Field
        label="Forbidden words in file content"
        hint="Words the lines a commit adds to its files may not carry (the pre-commit hook scans the staged diff). Empty = the message list above screens file content too. Set it narrower to let source name what a message may not, e.g. a file called after a vendor. A project's own message list still screens its content unless the project sets this list too."
      >
        <StringList
          items={value.forbidden_in_content ?? []}
          disabled={disabled}
          placeholder="e.g. the attribution trailer"
          emptyText="None — the message list above applies to file content too."
          onChange={(forbidden_in_content) => patch({ forbidden_in_content })}
        />
      </Field>

      <Field
        label="Commit style"
        hint="Rendered into every participant's prompt as the house commit style (subject mood, prefixes, footers) — described to the agent, not mechanically enforced. Empty = none."
      >
        <textarea
          value={value.commit_style ?? ""}
          disabled={disabled}
          rows={3}
          onChange={(e) => patch({ commit_style: e.target.value })}
          placeholder="e.g. imperative-mood, lowercase, conventional prefixes"
          className={cn(fieldClass, "min-h-[3.5rem] resize-y")}
        />
      </Field>

      <Field
        label="Round cap (laps)"
        hint="Safety net, not a checkpoint: the turn cycle halts and yields to you after this many full passes over the participants. Empty = inherit (500 laps). 0 = off."
      >
        <input
          type="number"
          min={0}
          step={1}
          value={value.round_cap ?? ""}
          disabled={disabled}
          // Empty box === `null` === "not set at this tier", which is a
          // different thing from 0 ("off"): `Number("")` is 0, so coercing
          // would silently disarm the cap on any edit that cleared the field.
          onChange={(e) =>
            patch({
              round_cap: e.target.value === "" ? null : Math.max(0, e.target.valueAsNumber || 0),
            })
          }
          placeholder="500 (inherited)"
          className={fieldClass}
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
        <span className="mt-1 block font-body-md text-body-md text-muted">
          {hint}
        </span>
      )}
    </label>
  );
}

function StringList({
  items,
  placeholder,
  emptyText = "None — nothing enforced for this list.",
  onChange,
  disabled,
}: {
  items: string[];
  placeholder?: string;
  /** What an empty list means — not always "nothing enforced". */
  emptyText?: string;
  onChange: (next: string[]) => void;
  disabled?: boolean;
}) {
  const { keys, replaceAt, removeAt, append } = useListEditor(items, onChange);

  return (
    <div className="flex flex-col gap-2">
      {items.length === 0 && (
        <p className="font-code-sm text-code-sm text-subtle">
          {emptyText}
        </p>
      )}
      {items.map((it, i) => (
        <div key={keys[i]} className="flex items-center gap-2">
          <input
            type="text"
            value={it}
            disabled={disabled}
            placeholder={placeholder}
            onChange={(e) => replaceAt(i, e.target.value)}
            className={cn(fieldClass, "flex-1")}
          />
          <Button
            variant="secondary"
            size="sm"
            disabled={disabled}
            onClick={() => removeAt(i)}
            aria-label="Remove entry"
            className="shrink-0"
          >
            <CloseIcon className="size-3.5" />
          </Button>
        </div>
      ))}
      <div>
        <Button variant="ghost" size="sm" onClick={() => append("")} disabled={disabled}>
          + Add
        </Button>
      </div>
    </div>
  );
}
