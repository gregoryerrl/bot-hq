import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { useTauriQuery } from "../hooks/useInvoke";
import { Button } from "./ui/Button";
import { formatRelative } from "../lib/time";
import type { UserActionItemView } from "../lib/bindings";

/**
 * "Your action items" — the session documents an agent marked
 * `kind: "user_action"` (feedback #89, the user's first slice): a draft to
 * post under the user's name, an entry only they can make, a command only they
 * can run. Session docs are archived with their session, so without this an
 * EOD draft written at 17:55 was findable only while the session stayed open.
 * Listed across every session, open or closed, until the user dismisses each
 * one; Open goes to the session, whose doc tab carries the item. No review step
 * in this slice.
 */
export function ActionItemsPanel() {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { data } = useTauriQuery<UserActionItemView[] | null>("list_user_action_items");
  // A null answer (a test stub, a failed read) is "no items", never a crash
  // on the dashboard.
  const items: UserActionItemView[] = Array.isArray(data) ? data : [];
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  if (items.length === 0) return null;

  const dismiss = async (item: UserActionItemView) => {
    const key = `${item.session_id}/${item.slug}`;
    setBusy(key);
    setError(null);
    try {
      await invoke("dismiss_user_action_item", { sessionId: item.session_id, slug: item.slug });
      await queryClient.invalidateQueries({ queryKey: ["list_user_action_items"] });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <section
      aria-label="Your action items"
      data-testid="action-items"
      className="mb-4 rounded-md border border-tertiary/40 bg-surface-container-low px-3 py-2"
    >
      <div className="mb-1 flex items-baseline gap-2">
        <h3 className="font-label-caps text-label-caps text-tertiary">Your action items</h3>
        <span className="text-xs text-on-surface-variant">
          {items.length === 1 ? "1 document an agent left for you to act on" : `${items.length} documents agents left for you to act on`}
        </span>
      </div>
      <ul className="divide-y divide-outline-variant/40">
        {items.map((item) => {
          const key = `${item.session_id}/${item.slug}`;
          return (
            <li key={key} className="flex items-center gap-3 py-1.5 text-sm">
              <div className="min-w-0 flex-1">
                <div className="flex items-baseline gap-2">
                  <code className="font-mono text-xs text-on-surface">{item.slug}</code>
                  <span className="truncate text-xs text-on-surface-variant" title={item.session_title}>
                    {item.session_title}
                    {item.session_closed_at ? " · closed" : ""}
                  </span>
                  <span className="shrink-0 text-xs text-on-surface-variant">
                    {formatRelative(item.updated_at)}
                  </span>
                </div>
                {item.preview && (
                  <p className="truncate text-xs text-on-surface-variant">{item.preview}</p>
                )}
              </div>
              <Button size="sm" variant="secondary" onClick={() => navigate(`/sessions/${item.session_id}?doc=${encodeURIComponent(item.slug)}`)}>
                Open
              </Button>
              <Button size="sm" variant="ghost" disabled={busy === key} onClick={() => void dismiss(item)}>
                Dismiss
              </Button>
            </li>
          );
        })}
      </ul>
      {error && <p className="mt-1 text-xs text-error">{error}</p>}
    </section>
  );
}
