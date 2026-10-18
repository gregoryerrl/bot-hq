import { useCallback, useEffect, useState } from "react";

/**
 * "Discard changes?" before an action that would drop unsaved edits — the CL
 * editor's close-tab pattern (`ContextLibraryEditor`), shared by the surfaces
 * that used to drop a half-written edit without a word: the Roles rail, the
 * session settings drawer, the Context tab's file list.
 *
 * `guard(go)` runs `go` at once when nothing is dirty, else holds it until the
 * user answers the dialog: `confirm` runs the held action, `cancel` drops it.
 * Render a `ConfirmDialog` with `open={asking}`.
 */
export function useDiscardGuard(dirty: boolean) {
  const [pending, setPending] = useState<(() => void) | null>(null);
  const guard = useCallback(
    (go: () => void) => {
      if (dirty) setPending(() => go);
      else go();
    },
    [dirty],
  );
  const confirm = useCallback(() => {
    const go = pending;
    setPending(null);
    go?.();
  }, [pending]);
  const cancel = useCallback(() => setPending(null), []);
  return { guard, asking: pending !== null, confirm, cancel };
}

/**
 * Report a child editor's dirty flag to the parent that guards switching away
 * from it — and report clean when the editor unmounts, so a parent never keeps
 * asking about an editor that is gone. Pass a stable `onDirtyChange` (a state
 * setter).
 */
export function useReportDirty(dirty: boolean, onDirtyChange?: (dirty: boolean) => void) {
  useEffect(() => {
    onDirtyChange?.(dirty);
  }, [dirty, onDirtyChange]);
  useEffect(() => () => onDirtyChange?.(false), [onDirtyChange]);
}
