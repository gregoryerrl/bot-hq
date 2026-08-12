import { useEffect, useMemo, useState } from "react";
import { useTauriQuery, useTauriMutation } from "../hooks/useInvoke";
import { Button } from "../components/ui/Button";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { cn } from "../lib/cn";
import { terminalInputClass, FieldLabel, SaveIcon } from "./contextLibraryShared";
import type {
  AppError,
  CapabilityView,
  ModelView,
  RoleDraftInput,
  RoleView,
} from "../lib/bindings";

const selectClass =
  "w-full rounded border border-outline-variant bg-surface-container-lowest px-2 py-1.5 font-code-sm text-code-sm text-on-surface focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary";

/**
 * The participation modes the picker OFFERS.
 *
 * `on_demand` exists in the column (0044) and in `PARTICIPATION_MODES`, and is
 * deliberately absent here — rc3 decision D1 makes an on-demand role wake on a
 * user `@mention`, and mention-wake is not built. Offering the mode would ship
 * a role that is enabled, listed in the roster, and never handed a turn, with
 * nothing to grep for.
 *
 * A role ALREADY stored as `on_demand` still renders — see `modeOptions`. The
 * picker refusing to show a value the row holds is how an edit to the prose
 * silently rewrites the mode.
 */
const OFFERED_MODES = [
  { value: "active", label: "Active — takes turns in the rotation" },
  { value: "observer", label: "Observer — reads the session, never wakes" },
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
 * HANDS and EYES are rows here like any other — seeded by migration 0044/0046
 * from the built-in constants, flagged `builtin` for provenance only. Nothing
 * on this tab is read-only because of that flag.
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
          <h1 className="font-headline-lg text-headline-lg text-on-surface">
            Roles
          </h1>
          <p className="mt-1 max-w-prose font-body-md text-body-md text-on-surface-variant">
            A role is a template a session's participants are invited from: the
            instruction it runs with, what it is allowed to do, and the model it
            defaults to. Add as many as you like — HANDS and EYES are just the
            two that ship seeded.
          </p>
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
          <ul className="min-h-0 flex-1 overflow-y-auto">
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
                    {r.builtin && (
                      <span className="shrink-0 rounded border border-secondary/50 px-1 font-label-caps text-label-caps text-secondary">
                        built-in
                      </span>
                    )}
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
        <div className="min-w-0 flex-1 overflow-y-auto rounded-lg border border-outline-variant bg-surface-container">
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
          <h2 className="truncate font-headline-md text-headline-md text-on-surface">
            {role ? role.display_name : "New role"}
          </h2>
          <p className="font-code-sm text-code-sm text-on-surface-variant">
            {role ? (
              <>
                <code className="text-on-surface-variant">{role.slug}</code>
                {role.builtin && " · seeded by bot-hq, and fully editable"}
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
        <div>
          {role && (
            <Button
              variant={role.archived ? "secondary" : "danger"}
              size="sm"
              disabled={archive.isPending}
              onClick={async () => {
                if (role.archived) {
                  // Restoring is not destructive — no confirmation.
                  await archive.mutateAsync({ id: role.id, archived: false });
                  onArchived();
                } else {
                  setConfirmArchive(true);
                }
              }}
            >
              {role.archived ? "Restore role" : "Archive role"}
            </Button>
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
          <button
            type="button"
            disabled={pending || !capsReady || !draft.display_name.trim()}
            onClick={submit}
            className="inline-flex items-center gap-1.5 rounded border border-primary bg-primary px-3 py-1.5 font-code-sm text-code-sm text-on-primary transition-colors hover:bg-primary-fixed disabled:opacity-50"
          >
            <SaveIcon />
            {pending ? "Saving…" : role ? "Save role" : "Create role"}
          </button>
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
