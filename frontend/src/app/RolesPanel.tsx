import { useEffect, useMemo, useState } from "react";
import { useTauriQuery, useTauriMutation } from "../hooks/useInvoke";
import { Button } from "../components/ui/Button";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { cn } from "../lib/cn";
import { terminalInputClass, FieldLabel } from "./contextLibraryShared";
import { SaveIcon } from "../components/icons";
import type {
  AppError,
  CapabilityView,
  ClaudeConfigView,
  ClaudeOverrides,
  ModelView,
  RoleDraftInput,
  RoleView,
} from "../lib/bindings";
import { selectClass } from "../components/ui/Select";

/**
 * The participation modes the picker OFFERS — **two, and both of them do
 * something** (rc3 D18).
 *
 * `observer` was the third and is gone: it was spawned, handed no turn,
 * delivered nothing and could not vote, so it read nothing, said nothing and
 * billed for existing. What it was reached for — a role that watches and speaks
 * rarely — is `on_mention`.
 *
 * A role stored with some OTHER value still renders; see `modeOptions`. A picker
 * that refuses to show the value a row holds is how an edit to the prose
 * silently rewrites the mode.
 */
const OFFERED_MODES = [
  { value: "active", label: "Active — takes turns in the rotation" },
  {
    value: "on_mention",
    label: "On mention — waits; takes a turn when you @ it",
  },
] as const;

const NO_MODEL = "__none__";

/** Blank slate for "+ New role". Capabilities start empty: a grant is a
 * decision, and pre-ticking boxes makes it one the user never made. */
function emptyDraft(): RoleDraftInput {
  return {
    display_name: "",
    slug: null,
    description_prompt: "",
    capabilities: [],
    participation_mode: "active",
    default_model_id: null,
  };
}

function draftOf(role: RoleView): RoleDraftInput {
  return {
    display_name: role.display_name,
    // `null` on update means "leave the slug alone" — NOT "unset it". A rename
    // has to be explicit, because `ensure_session_roster` looks the seeded
    // roles up by the literal slugs `hands` / `eyes`.
    slug: null,
    description_prompt: role.description_prompt ?? "",
    capabilities: role.capabilities,
    participation_mode: role.participation_mode,
    default_model_id: role.default_model_id,
  };
}

/**
 * Settings → Roles. The user's own role templates: each one carries the prose
 * that gets injected into a session, the capabilities it grants, how it
 * participates, and which model it defaults to (rc3 decision D8).
 *
 * Roles are entirely user-owned rows (a fresh 1.0.0 install seeds one neutral
 * `agent` role; the HANDS/EYES pair is a one-time offer, not furniture).
 * `builtin` is permanently false and nothing on this tab reads it.
 *
 * Master/detail rather than a modal: the role instruction is the point of the
 * tab and a long-form editor does not fit in a dialog. The list is the rail;
 * the pane beside it is one role's full form.
 */
export function RolesPanel() {
  const [includeArchived, setIncludeArchived] = useState(false);
  const {
    data: roles = [],
    refetch,
    isLoading,
    error: listError,
  } = useTauriQuery<RoleView[]>(
    "list_roles",
    { includeArchived },
    // Toggling "Show archived" changes the query KEY, so without this the list
    // is `undefined` until the new fetch lands — the selected role vanishes for
    // a frame, the editor beside it unmounts, and a half-written instruction
    // goes with it. Holding the previous page keeps the pane mounted.
    { placeholderData: (prev) => prev },
  );
  const { data: models = [] } = useTauriQuery<ModelView[]>("list_models");
  const { data: capabilities = [] } =
    useTauriQuery<CapabilityView[]>("list_capabilities");
  // The one-time example-pair offer (Batch 4). Only the literal 'pending'
  // renders the card; null / absent / any other value is silence.
  const { data: presetOffer = null, refetch: refetchOffer } = useTauriQuery<
    string | null
  >("get_app_setting", { key: "role_preset_offer" });
  const resolveOffer = useTauriMutation<null, { install: boolean }>(
    "resolve_role_preset_offer",
  );
  const answerOffer = (install: boolean) =>
    resolveOffer.mutate(
      { install },
      {
        onSuccess: () => {
          void refetchOffer();
          void refetch();
        },
      },
    );

  // `null` = the "+ New role" form; a number = that role's id.
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [creating, setCreating] = useState(false);

  const selected = roles.find((r) => r.id === selectedId) ?? null;

  // Land on something as soon as the list arrives, so the pane is never a
  // blank panel the user has to discover a click for. It also catches the row
  // going away — archiving clears the selection, and this picks the next one.
  //
  // The condition is "nothing selected RESOLVES to a row", so a selected id
  // that is not in `roles` yet counts as nothing. That is why creating a role
  // refetches BEFORE it selects the new id: select first and this fallback
  // fires in the gap and lands the user on somebody else's role.
  useEffect(() => {
    if (creating || selected || roles.length === 0) return;
    setSelectedId(roles[0].id);
  }, [creating, selected, roles]);

  return (
    <div className="mx-auto flex h-full max-w-7xl flex-col overflow-hidden px-6 py-6">
      <div className="mb-4 flex shrink-0 items-start justify-between gap-4">
        <div>
          <h2 className="font-headline-lg text-headline-lg text-on-surface">
            Roles
          </h2>
          <p className="mt-1 max-w-prose font-body-md text-body-md text-on-surface-variant">
            A role is a template a session's participants are invited from: the
            instruction it runs with, what it is allowed to do, and the model it
            defaults to. A session picks its participants from the roles you
            define here — add as many as you like.
          </p>
          {/* The ONE-TIME example-pair offer (1.0.0 Batch 4, the user's
              design): rendered only while the flag is the literal 'pending'
              — seeded by migration 0072 on FRESH installs only. An absent
              key is NO offer (EYES E4), so an upgrading install never sees
              this; declining stamps 'declined' and it never returns. */}
          {presetOffer === "pending" && (
            <div className="mt-3 max-w-prose rounded-md border border-outline-variant bg-surface-container-low p-3">
              <p className="font-body-md text-body-md text-on-surface">
                Want a starting point? Install the example pair — an executor
                (HANDS) and an adversarial reviewer (EYES) with battle-tested
                instructions. They become ordinary roles you can edit or
                remove.
              </p>
              <div className="mt-2 flex gap-2">
                <Button
                  variant="primary"
                  onClick={() => answerOffer(true)}
                  disabled={resolveOffer.isPending}
                >
                  Install pair
                </Button>
                <Button
                  variant="ghost"
                  onClick={() => answerOffer(false)}
                  disabled={resolveOffer.isPending}
                >
                  No thanks
                </Button>
              </div>
            </div>
          )}
        </div>
        <Button
          variant="primary"
          onClick={() => {
            setCreating(true);
            setSelectedId(null);
          }}
        >
          + New role
        </Button>
      </div>

      {listError && (
        <p
          role="alert"
          className="mb-3 shrink-0 rounded border border-error/40 bg-error-container/20 px-3 py-2 font-code-sm text-code-sm text-on-error-container"
        >
          Couldn&rsquo;t load roles: {listError.message}
        </p>
      )}

      <div className="flex min-h-0 flex-1 gap-gutter">
        {/* ---- rail ---- */}
        <div className="flex w-64 shrink-0 flex-col overflow-hidden rounded-lg border border-outline-variant bg-surface-container">
          <div className="shrink-0 border-b border-outline-variant px-3 py-2">
            <label className="flex items-center gap-2">
              <input
                type="checkbox"
                checked={includeArchived}
                onChange={(e) => setIncludeArchived(e.target.checked)}
                className="size-4 accent-primary"
              />
              <span className="font-code-sm text-code-sm text-on-surface-variant">
                Show archived
              </span>
            </label>
          </div>
          <ul className="min-h-0 flex-1 overflow-y-auto overflow-x-hidden">
            {creating && (
              <li>
                <button
                  type="button"
                  onClick={() => setSelectedId(null)}
                  className="flex w-full items-center gap-2 border-l-2 border-primary bg-surface-container-high px-3 py-2 text-left"
                >
                  <span className="truncate font-code-sm text-code-sm text-primary">
                    New role…
                  </span>
                </button>
              </li>
            )}
            {isLoading ? (
              <li className="px-3 py-2 font-code-sm text-code-sm text-on-surface-variant">
                Loading…
              </li>
            ) : roles.length === 0 ? (
              <li className="px-3 py-3 font-code-sm text-code-sm text-on-surface-variant">
                No roles yet.
              </li>
            ) : (
              roles.map((r) => (
                <li key={r.id}>
                  <button
                    type="button"
                    onClick={() => {
                      setCreating(false);
                      setSelectedId(r.id);
                    }}
                    className={cn(
                      "flex w-full items-center gap-2 border-l-2 px-3 py-2 text-left transition-colors",
                      r.id === selectedId && !creating
                        ? "border-primary bg-surface-container-high"
                        : "border-transparent hover:bg-surface-container-high/60",
                    )}
                  >
                    <span className="min-w-0 flex-1">
                      <span className="block truncate font-body-md text-body-md text-on-surface">
                        {r.display_name}
                      </span>
                      <span className="block truncate font-code-sm text-code-sm text-on-surface-variant">
                        {r.slug}
                      </span>
                    </span>
                    {/* No "built-in" chip: migration 0048 set `builtin = 0` on
                        every row to state that bot-hq ships no roles, and
                        nothing writes it back, so the chip could never render
                        again. */}
                    {r.archived && (
                      <span className="shrink-0 rounded border border-outline-variant/60 px-1 font-label-caps text-label-caps text-on-surface-variant">
                        archived
                      </span>
                    )}
                  </button>
                </li>
              ))
            )}
          </ul>
        </div>

        {/* ---- detail ---- */}
        <div className="min-w-0 flex-1 overflow-y-auto overflow-x-hidden rounded-lg border border-outline-variant bg-surface-container">
          {creating && selectedId === null ? (
            <RoleForm
              key="new"
              role={null}
              models={models}
              capabilities={capabilities}
              onSaved={async (saved) => {
                // Refetch BEFORE switching, so the new role is already in the
                // rail when the form remounts against it.
                await refetch();
                setCreating(false);
                setSelectedId(saved.id);
              }}
              onArchived={() => {}}
              onCancelCreate={() => setCreating(false)}
            />
          ) : selected ? (
            <RoleForm
              key={selected.id}
              role={selected}
              models={models}
              capabilities={capabilities}
              onSaved={async () => {
                await refetch();
              }}
              onArchived={() => {
                setSelectedId(null);
                refetch();
              }}
              onCancelCreate={() => {}}
            />
          ) : (
            <p className="px-5 py-6 font-code-sm text-code-sm text-on-surface-variant">
              Pick a role from the list, or add one.
            </p>
          )}
        </div>
      </div>
    </div>
  );
}

// ============================================================================
// RoleForm — one role's full editor. `role === null` is the create form.
// ============================================================================

function RoleForm({
  role,
  models,
  capabilities,
  onSaved,
  onArchived,
  onCancelCreate,
}: {
  role: RoleView | null;
  models: ModelView[];
  capabilities: CapabilityView[];
  onSaved: (saved: RoleView) => void | Promise<void>;
  onArchived: () => void;
  onCancelCreate: () => void;
}) {
  // Seeded ONCE, from the row this form was mounted for — the parent keys the
  // form by role id, so switching roles remounts it.
  //
  // Deliberately not an effect that re-seeds whenever the server row's
  // identity changes: `list_roles` is a React Query read that refetches in the
  // background (window focus, invalidation), and re-seeding on that would
  // silently discard a half-written role instruction. `baseline` moves only
  // when a save returns a new stored row.
  const [baseline, setBaseline] = useState<RoleDraftInput>(() =>
    role ? draftOf(role) : emptyDraft(),
  );
  const [draft, setDraft] = useState<RoleDraftInput>(() =>
    role ? draftOf(role) : emptyDraft(),
  );
  const [error, setError] = useState<AppError | null>(null);
  const [confirmArchive, setConfirmArchive] = useState(false);
  const [saved, setSaved] = useState(false);

  // Default effort (hotfix 2026-08-25): a Roles-tab surface over the per-role
  // Claude-Config override — the slot spawn resolves and the dialog's
  // "Inherit (…)" already reads. Read-modify-write of the whole store via the
  // existing set_claude_overrides; refetch keeps every other reader in sync.
  const { data: claudeOverrides, refetch: refetchOverrides } =
    useTauriQuery<ClaudeOverrides>("get_claude_overrides");
  const { data: claudeConfig } =
    useTauriQuery<ClaudeConfigView>("claude_config_read");
  const setOverrides = useTauriMutation<null, { overrides: ClaudeOverrides }>(
    "set_claude_overrides",
  );
  const [savingEffort, setSavingEffort] = useState(false);
  const roleEffort = role
    ? (claudeOverrides?.per_role?.[role.slug]?.effort ?? null)
    : null;
  // What Inherit falls to — _all's effort, else the settings.json env knob —
  // mirroring resolve_agent_overrides + the spawn env fall-through.
  const inheritedEffortNote =
    claudeOverrides?._all?.effort ??
    claudeConfig?.core_knobs.find(
      (k) => k.key === "env.CLAUDE_CODE_EFFORT_LEVEL",
    )?.value ??
    null;
  const saveRoleEffort = async (value: string | null) => {
    if (!role) return;
    setSavingEffort(true);
    try {
      const perRole = { ...(claudeOverrides?.per_role ?? {}) };
      const entry = { ...(perRole[role.slug] ?? {}) };
      entry.effort = value;
      perRole[role.slug] = entry;
      const store: ClaudeOverrides = {
        ...(claudeOverrides ?? {}),
        per_role: perRole,
      };
      await setOverrides.mutateAsync({ overrides: store });
      await refetchOverrides();
    } catch (e) {
      setError(e as AppError);
    } finally {
      setSavingEffort(false);
    }
  };

  const create = useTauriMutation<RoleView, { draft: RoleDraftInput }>(
    "create_role",
  );
  const update = useTauriMutation<
    RoleView,
    { id: number; draft: RoleDraftInput }
  >("update_role");
  const archive = useTauriMutation<void, { id: number; archived: boolean }>(
    "archive_role",
  );

  const dirty = JSON.stringify(draft) !== JSON.stringify(baseline);
  const pending = create.isPending || update.isPending;
  // The same `.trim() === ""` reading `submit` uses to decide `null`, so what
  // the notice below promises and what the save actually sends are one test.
  const promptCleared = (draft.description_prompt ?? "").trim() === "";

  // Slugs the checklist knows about. Anything on the role that is NOT in here
  // is shown separately rather than as a silent omission — see below.
  const known = useMemo(
    () => new Set(capabilities.map((c) => c.slug)),
    [capabilities],
  );
  // Saving rewrites `capabilities` from the checklist, so with no checklist the
  // save would strip every grant the role has. Until `list_capabilities`
  // answers, the form cannot know what a tickless box means, and guessing is
  // the one outcome that silently destroys a configuration.
  const capsReady = capabilities.length > 0;
  const unrecognised = capsReady
    ? draft.capabilities.filter((s) => !known.has(s))
    : [];

  // Group the checklist, preserving the order the backend sent.
  const groups = useMemo(() => {
    const out: { name: string; rows: CapabilityView[] }[] = [];
    for (const cap of capabilities) {
      const g = out.find((x) => x.name === cap.group);
      if (g) g.rows.push(cap);
      else out.push({ name: cap.group, rows: [cap] });
    }
    return out;
  }, [capabilities]);

  const toggle = (slug: string, on: boolean) =>
    setDraft((d) => ({
      ...d,
      capabilities: on
        ? [...d.capabilities, slug]
        : d.capabilities.filter((s) => s !== slug),
    }));

  // A role stored as `on_demand` keeps its option so the picker cannot rewrite
  // it by omission — but it is disabled, because the mode does not run yet.
  const modeOptions = OFFERED_MODES.some(
    (m) => m.value === draft.participation_mode,
  )
    ? null
    : draft.participation_mode;

  // `Validation` is the backend's field-level refusal — its doc says the
  // frontend highlights the offending field, and everything else is a toast.
  // The two shapes it produces about capabilities are `unknown capabilit…` (an
  // unparseable slug) and "`a` requires `b`" (an incoherent set), so both point
  // at the checklist.
  const validation = error?.kind === "Validation" ? error.message : null;
  const nameInvalid = validation !== null && /name/i.test(validation);
  const capabilityInvalid =
    validation !== null &&
    (/capabilit/i.test(validation) || / requires /.test(validation));

  const submit = async () => {
    setError(null);
    // Trailing whitespace in a prompt is invisible; an all-whitespace prompt is
    // not the same as no prompt, and storing `""` would make "has an
    // instruction" untestable downstream. Empty becomes NULL.
    const prompt = (draft.description_prompt ?? "").trim();
    const payload: RoleDraftInput = {
      ...draft,
      description_prompt: prompt === "" ? null : draft.description_prompt,
      // Unrecognised slugs are dropped on save — the checklist is the whole
      // truth of what is granted, and the backend refuses a slug it cannot
      // parse, so keeping them would make the role unsaveable forever.
      capabilities: draft.capabilities.filter((s) => known.has(s)),
    };
    try {
      const result = role
        ? await update.mutateAsync({ id: role.id, draft: payload })
        : await create.mutateAsync({ draft: payload });
      // Re-seed from the STORED row, not from the draft: create allocates the
      // slug (a collision suffixes it) and the trims land server-side, so the
      // form would otherwise keep claiming unsaved changes forever.
      setBaseline(draftOf(result));
      setDraft(draftOf(result));
      setSaved(true);
      await onSaved(result);
    } catch (e) {
      setError(e as AppError);
    }
  };

  return (
    <div className="flex flex-col gap-5 px-5 py-5">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="truncate font-headline-md text-headline-md text-on-surface">
            {role ? role.display_name : "New role"}
          </h3>
          <p className="font-code-sm text-code-sm text-on-surface-variant">
            {role ? (
              <>
                <code className="text-on-surface-variant">{role.slug}</code>
                {/* The " · seeded by bot-hq" suffix went with the rail's
                    "built-in" chip, and for the same reason: `builtin` is 0 on
                    every row after 0048, so it was dead. */}
                {role.archived && " · archived"}
              </>
            ) : (
              "The slug is derived from the display name when you save."
            )}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {dirty && (
            <span className="rounded border border-warning/40 bg-warning/15 px-1.5 py-0.5 font-label-caps text-label-caps text-warning">
              Unsaved
            </span>
          )}
          {saved && !dirty && (
            <span className="rounded border border-success/40 bg-success/15 px-1.5 py-0.5 font-label-caps text-label-caps text-success">
              Saved ✓
            </span>
          )}
        </div>
      </div>

      <label className="block">
        <FieldLabel>Display name</FieldLabel>
        <input
          type="text"
          value={draft.display_name}
          aria-invalid={nameInvalid || undefined}
          onChange={(e) => setDraft({ ...draft, display_name: e.target.value })}
          placeholder="e.g. Code Reviewer"
          className={cn(terminalInputClass, nameInvalid && "border-error")}
        />
      </label>

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
        <label className="block">
          <FieldLabel>Participation mode</FieldLabel>
          <select
            value={draft.participation_mode}
            onChange={(e) =>
              setDraft({ ...draft, participation_mode: e.target.value })
            }
            className={selectClass}
          >
            {OFFERED_MODES.map((m) => (
              <option key={m.value} value={m.value}>
                {m.label}
              </option>
            ))}
            {modeOptions && (
              <option value={modeOptions} disabled>
                {modeOptions} — not selectable yet
              </option>
            )}
          </select>
          {modeOptions && (
            <span className="mt-1 block font-code-sm text-code-sm text-warning">
              This role is stored as <code>{modeOptions}</code>, which bot-hq
              cannot schedule yet. Pick another mode to make it run.
            </span>
          )}
        </label>

        <label className="block">
          <FieldLabel>Default model</FieldLabel>
          <select
            value={draft.default_model_id ?? NO_MODEL}
            onChange={(e) =>
              setDraft({
                ...draft,
                default_model_id:
                  e.target.value === NO_MODEL ? null : e.target.value,
              })
            }
            className={selectClass}
          >
            <option value={NO_MODEL}>
              (none — chosen when the role is invited)
            </option>
            {models.map((m) => (
              <option key={m.id} value={m.id}>
                {m.display_name}
                {m.model_name ? ` — ${m.model_name}` : ""}
              </option>
            ))}
          </select>
          <span className="mt-1 block font-code-sm text-code-sm text-on-surface-variant">
            {models.length === 0
              ? "No saved models yet — add them in the Models tab."
              : "The New-session dialog can override this per participant."}
          </span>
        </label>

        {/* Default EFFORT (hotfix, user 2026-08-25). This is a surface over
            the SAME slot spawn already resolves — `claude-overrides.json`
            per_role[slug].effort — not a new roles column, so the New-session
            dialog's "Inherit (…)" label, the spawn chain, and the Claude
            Config tab all agree by construction. Written on change (it's a
            settings knob, not part of the role-row draft); hidden on the
            create form until the role exists to key the slot. */}
        {role && (
          <label className="block">
            <FieldLabel>Default effort</FieldLabel>
            <select
              value={roleEffort ?? ""}
              disabled={savingEffort}
              onChange={(e) => void saveRoleEffort(e.target.value || null)}
              className={selectClass}
              aria-label="Default effort"
            >
              <option value="">
                Inherit{inheritedEffortNote ? ` (${inheritedEffortNote})` : ""}
              </option>
              {["low", "medium", "high", "xhigh", "max"].map((v) => (
                <option key={v} value={v}>
                  {v}
                </option>
              ))}
            </select>
            <span className="mt-1 block font-code-sm text-code-sm text-on-surface-variant">
              What this role's participants spawn with unless the New-session
              dialog picks otherwise. Stored in Claude Config's per-role
              overrides; sessions record the resolved value on their spawn
              badge.
            </span>
          </label>
        )}
      </div>

      <label className="block">
        <FieldLabel>Role instruction</FieldLabel>
        <textarea
          value={draft.description_prompt ?? ""}
          onChange={(e) =>
            setDraft({ ...draft, description_prompt: e.target.value })
          }
          spellCheck={false}
          rows={18}
          aria-label="Role instruction"
          placeholder="Markdown. The role's identity, voice and priorities — injected into every session this role joins."
          className="min-h-[22rem] w-full resize-y whitespace-pre-wrap break-words rounded border border-outline-variant bg-surface-container-lowest px-3 py-2 font-code-sm text-code-sm leading-relaxed text-on-surface caret-primary placeholder:text-on-surface-variant focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
        />
        <span className="mt-1 block max-w-prose font-code-sm text-code-sm text-on-surface-variant">
          Markdown, injected into every session this role joins. It is the
          role&rsquo;s identity layer only — bot-hq&rsquo;s core rules and the
          rules derived from the capabilities below are composed after it at
          spawn, and the capabilities themselves are what the tool gate reads.
          Text written here cannot grant a capability the boxes below withhold.
        </span>
        {/* Clearing means CLEARED (1.0.0 Batch 4): the compiled-prose
            fallback was deleted with the neutral default role, so an empty
            box stores no prose and the role joins sessions with no
            instruction of its own — briefed by the universal rules and its
            capability grants alone. One arm, no built-in-prose branch —
            the fallback (and its flag) died in Batches 4/6. */}
        {promptCleared && (
          <span
            role="note"
            className="mt-1 block max-w-prose font-code-sm text-code-sm text-warning"
          >
            Empty means empty: saving stores no prose, and this role joins
            sessions with no instruction of its own (its capabilities and the
            universal rules still apply).
          </span>
        )}
      </label>

      <fieldset
        className={cn(
          "rounded border p-4",
          capabilityInvalid ? "border-error" : "border-outline-variant/60",
        )}
      >
        <legend className="px-1 font-label-caps text-label-caps text-on-surface-variant">
          Capabilities
        </legend>
        <p className="mb-3 max-w-prose font-code-sm text-code-sm text-on-surface-variant">
          Grants only — there is no separate deny list to contradict, so
          anything unticked is simply not granted. Most boxes are enforced when
          the agent calls the matching tool; <em>Edit files</em> is enforced at
          spawn, by how the agent&rsquo;s process is launched.{" "}
          <em>Read the channel</em>, <em>Post to the channel</em> and{" "}
          <em>Run Bash</em> are described to the agent but not yet mechanically
          enforced.
        </p>
        {!capsReady ? (
          <p className="font-code-sm text-code-sm text-warning">
            Loading the capability list — saving is held until it arrives, so a
            save can&rsquo;t strip grants this form hasn&rsquo;t seen.
          </p>
        ) : (
          <div className="flex flex-col gap-4">
            {groups.map((g) => (
              <div key={g.name}>
                <p className="mb-1.5 font-label-caps text-label-caps text-on-surface-variant/70">
                  {g.name}
                </p>
                <div className="grid grid-cols-1 gap-1.5 lg:grid-cols-2">
                  {g.rows.map((cap) => {
                    const on = draft.capabilities.includes(cap.slug);
                    const missing = on
                      ? cap.requires.filter(
                          (dep) => !draft.capabilities.includes(dep),
                        )
                      : [];
                    return (
                      <label
                        key={cap.slug}
                        className="flex items-start gap-2 rounded px-1 py-1 hover:bg-surface-container-high/50"
                      >
                        <input
                          type="checkbox"
                          checked={on}
                          onChange={(e) => toggle(cap.slug, e.target.checked)}
                          className="mt-1 size-4 shrink-0 accent-primary"
                        />
                        <span className="min-w-0">
                          <span className="block font-body-md text-body-md text-on-surface">
                            {cap.label}
                          </span>
                          <span className="block font-code-sm text-code-sm text-on-surface-variant">
                            {cap.description}
                          </span>
                          {missing.length > 0 && (
                            <span className="block font-code-sm text-code-sm text-warning">
                              needs {missing.join(", ")}
                            </span>
                          )}
                        </span>
                      </label>
                    );
                  })}
                </div>
              </div>
            ))}
          </div>
        )}
        {unrecognised.length > 0 && (
          <p className="mt-4 rounded border border-warning/40 bg-warning/10 px-3 py-2 font-code-sm text-code-sm text-warning">
            This role also stores {unrecognised.length} slug
            {unrecognised.length === 1 ? "" : "s"} bot-hq no longer recognises (
            <code>{unrecognised.join(", ")}</code>). They grant nothing today
            and saving removes them.
          </p>
        )}
      </fieldset>

      {error && (
        <p
          role="alert"
          className="rounded border border-error/40 bg-error-container/20 px-3 py-2 font-code-sm text-code-sm text-on-error-container"
        >
          {validation ? `Can’t save: ${validation}` : `Save failed: ${error.message}`}
        </p>
      )}

      <div className="flex items-center justify-between gap-2 border-t border-outline-variant/30 pt-4">
        <div className="flex flex-col items-start gap-2">
          {role && (
            <Button
              variant={role.archived ? "secondary" : "danger"}
              size="sm"
              disabled={archive.isPending}
              onClick={async () => {
                if (role.archived) {
                  // Restoring is not destructive, so it gets no confirmation —
                  // but it still has to REPORT. Awaiting the mutation bare left
                  // the rejection unhandled and the screen unchanged: the role
                  // simply stayed archived, which is indistinguishable from the
                  // click not registering. The archive path below already wraps
                  // and renders; this is the same treatment, and the asymmetry
                  // was the bug.
                  try {
                    await archive.mutateAsync({ id: role.id, archived: false });
                    onArchived();
                  } catch {
                    // Rendered by the alert below — the form stays put so the
                    // message has somewhere to sit.
                  }
                } else {
                  setConfirmArchive(true);
                }
              }}
            >
              {role.archived ? "Restore role" : "Archive role"}
            </Button>
          )}
          {/* Scoped to the archived case on purpose: the only way to reach
              `archive.error` while `!role.archived` is the confirm dialog,
              which renders it itself. Gating here keeps one failure from
              showing up in two places. */}
          {role?.archived && archive.error && (
            <p
              role="alert"
              className="rounded border border-error/40 bg-error-container/20 px-3 py-2 font-code-sm text-code-sm text-on-error-container"
            >
              Restore failed: {archive.error.message}
            </p>
          )}
        </div>
        <div className="flex shrink-0 gap-2">
          <Button
            variant="ghost"
            size="sm"
            disabled={pending}
            onClick={() => {
              if (role) {
                setDraft(baseline);
                setError(null);
              } else {
                onCancelCreate();
              }
            }}
          >
            {role ? "Reset" : "Cancel"}
          </Button>
          <Button
            type="button"
            variant="primary"
            disabled={pending || !capsReady || !draft.display_name.trim()}
            onClick={submit}
          >
            <SaveIcon />
            {pending ? "Saving…" : role ? "Save role" : "Create role"}
          </Button>
        </div>
      </div>

      <ConfirmDialog
        open={confirmArchive}
        title="Archive this role?"
        message={
          <>
            Archive{" "}
            <strong className="text-on-surface">
              {role?.display_name || "this role"}
            </strong>
            ? It leaves the list and can no longer be invited into a new
            session. Nothing is deleted — every past session keeps its record of
            having used it, and <strong>Show archived</strong> brings it back.
            {archive.error && (
              <span className="mt-3 block rounded border border-error/40 bg-error-container/20 px-3 py-2 text-on-error-container">
                Archive failed: {archive.error.message}
              </span>
            )}
          </>
        }
        confirmLabel="Archive"
        confirmVariant="danger"
        onConfirm={async () => {
          if (!role) return;
          try {
            await archive.mutateAsync({ id: role.id, archived: true });
            setConfirmArchive(false);
            onArchived();
          } catch {
            // Keep the dialog open so the inline error above is visible.
          }
        }}
        onCancel={() => setConfirmArchive(false)}
      />
    </div>
  );
}
