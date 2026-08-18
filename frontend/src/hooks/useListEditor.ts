import { useRef } from "react";

let rowSeq = 0;
const nextRowKey = () => `row-${++rowSeq}`;

/**
 * Immutable controlled-list edit helpers for a `value` + `onChange` pair. Shared
 * by the policy StringList and the Tool-Gate keyword rows, which both hand-rolled
 * the same map-by-index / filter-remove / spread-append idiom.
 *
 * `keys` gives each row a stable identity for React's `key` (round 11). The
 * rows are plain values with no id of their own, and keying them by index
 * meant a removal handed the DOM node — its focus, selection and IME state —
 * to whichever row inherited the index. The keys live in a ref and are kept
 * in step here: appends and removals move them with the row; an external
 * replacement of the whole list (a reset, a reload) is reconciled by length,
 * which is the best identity a value list can offer.
 */
export function useListEditor<T>(items: T[], onChange: (next: T[]) => void) {
  const keysRef = useRef<string[]>([]);
  const keys = keysRef.current;
  while (keys.length < items.length) keys.push(nextRowKey());
  if (keys.length > items.length) keys.length = items.length;
  return {
    keys,
    replaceAt: (i: number, item: T) =>
      onChange(items.map((it, idx) => (idx === i ? item : it))),
    removeAt: (i: number) => {
      keys.splice(i, 1);
      onChange(items.filter((_, idx) => idx !== i));
    },
    append: (item: T) => {
      keys.push(nextRowKey());
      onChange([...items, item]);
    },
  };
}
