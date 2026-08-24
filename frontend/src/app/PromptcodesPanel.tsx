import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTauriQuery, errorMessage } from "../hooks/useInvoke";
import { useListEditor } from "../hooks/useListEditor";
import { ErrorBanner } from "../components/ErrorBanner";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import { Textarea } from "../components/ui/Textarea";
import { Skeleton } from "../components/ui/Skeleton";

/** One code → prompt pair, as stored. */
interface Promptcode {
  code: string;
  prompt: string;
}

/** The `app_settings` key the composer's `/` picker reads. */
const SETTING_KEY = "promptcodes";

function parseCodes(raw: string | null | undefined): Promptcode[] {
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (c): c is Promptcode =>
        typeof c === "object" &&
        c !== null &&
        typeof (c as { code?: unknown }).code === "string" &&
        typeof (c as { prompt?: unknown }).prompt === "string",
    );
  } catch {
    return [];
  }
}

/**
 * Settings → Promptcodes (ideas.md, 2026-08-24): code → prompt pairs the
 * composer's `/` picker expands. "I have `/n-verify`, and its prompt is 'Do n
 * rounds of verification…'. I use that a lot, so instead of typing the whole
 * thing, i'll just type `/n-verify`." Picking a code replaces the token with
 * the FULL prompt text in the box — transparent, nothing expands at send time.
 *
 * Stored as one JSON list under `app_settings` ("promptcodes") via the generic
 * `get_app_setting`/`set_app_setting` pair — no schema, no migration.
 */
export function PromptcodesPanel() {
  const {
    data: raw = null,
    isLoading,
    refetch,
  } = useTauriQuery<string | null>("get_app_setting", { key: SETTING_KEY });

  const [rows, setRows] = useState<Promptcode[]>([]);
  const [seeded, setSeeded] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Seed the editor from storage ONCE per load; after that the draft is the
  // user's and a background refetch must not clobber it (same dirty-vs-fresh
  // rule as the CL editor).
  useEffect(() => {
    if (!seeded && !isLoading) {
      setRows(parseCodes(raw));
      setSeeded(true);
    }
  }, [seeded, isLoading, raw]);

  const edit = (next: Promptcode[]) => {
    setRows(next);
    setDirty(true);
  };
  const { keys, replaceAt, removeAt, append } = useListEditor(rows, edit);

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      const cleaned = rows.filter((r) => r.code.trim() !== "");
      await invoke("set_app_setting", {
        key: SETTING_KEY,
        value: JSON.stringify(cleaned),
      });
      setRows(cleaned);
      setDirty(false);
      await refetch();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="h-full overflow-y-auto overflow-x-hidden p-6">
      <div className="mx-auto max-w-3xl">
        <h2 className="font-headline-md text-headline-md text-on-surface">
          Promptcodes
        </h2>
        <p className="mt-1 text-sm text-on-surface-variant">
          Type <code className="font-mono">/code</code> in a session composer
          and the picker replaces it with the full prompt — the box always
          shows exactly what will send.
        </p>

        {error && (
          <ErrorBanner
            label="Save failed:"
            message={error}
            onDismiss={() => setError(null)}
            className="mt-3"
          />
        )}

        {!seeded ? (
          <Skeleton rows={2} className="mt-4 space-y-2" rowClassName="h-9 rounded border border-outline-variant bg-surface-container" />
        ) : (
          <>
            <ul className="mt-4 space-y-3">
              {rows.map((row, i) => (
                <li
                  key={keys[i]}
                  className="rounded border border-outline-variant bg-surface-container-low p-3"
                >
                  <div className="flex items-center gap-2">
                    <span className="font-mono text-sm text-on-surface-variant">
                      /
                    </span>
                    <Input
                      value={row.code}
                      onChange={(e) =>
                        replaceAt(i, { ...row, code: e.target.value.trim() })
                      }
                      placeholder="code (e.g. n-verify)"
                      aria-label={`Code ${i + 1}`}
                      className="max-w-xs font-mono"
                    />
                    <Button
                      type="button"
                      variant="ghost"
                      onClick={() => removeAt(i)}
                      aria-label={`Delete /${row.code || "code"}`}
                      className="ml-auto"
                    >
                      Delete
                    </Button>
                  </div>
                  <Textarea
                    value={row.prompt}
                    onChange={(e) =>
                      replaceAt(i, { ...row, prompt: e.target.value })
                    }
                    placeholder="The prompt this code expands to…"
                    aria-label={`Prompt for /${row.code || "code"}`}
                    rows={3}
                    className="mt-2 w-full resize-y"
                  />
                </li>
              ))}
            </ul>
            {rows.length === 0 && (
              <p className="mt-4 text-sm text-on-surface-variant">
                No promptcodes yet — add one below.
              </p>
            )}
            <div className="mt-4 flex items-center gap-2">
              <Button
                type="button"
                variant="secondary"
                onClick={() => append({ code: "", prompt: "" })}
              >
                Add promptcode
              </Button>
              <Button
                type="button"
                variant="primary"
                onClick={() => void save()}
                disabled={!dirty || saving}
              >
                {saving ? "Saving…" : dirty ? "Save" : "Saved"}
              </Button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
