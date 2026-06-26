/**
 * Immutable controlled-list edit helpers for a `value` + `onChange` pair. Shared
 * by the policy StringList and the Tool-Gate keyword rows, which both hand-rolled
 * the same map-by-index / filter-remove / spread-append idiom.
 */
export function useListEditor<T>(items: T[], onChange: (next: T[]) => void) {
  return {
    replaceAt: (i: number, item: T) =>
      onChange(items.map((it, idx) => (idx === i ? item : it))),
    removeAt: (i: number) => onChange(items.filter((_, idx) => idx !== i)),
    append: (item: T) => onChange([...items, item]),
  };
}
