import { describe, it, expect } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useState } from "react";
import { useListEditor } from "./useListEditor";

// Round 11: rows are values with no id, and `key={i}` handed a removed row's
// DOM node (focus, selection, IME) to whichever row inherited the index. The
// hook now issues stable keys that move WITH the row.
describe("useListEditor keys", () => {
  function useHarness(initial: string[]) {
    const [items, setItems] = useState(initial);
    return { items, ...useListEditor(items, setItems) };
  }

  it("keeps a surviving row's key across a removal before it", () => {
    const { result } = renderHook(() => useHarness(["a", "b", "c"]));
    const [ka, kb, kc] = result.current.keys;
    expect(new Set([ka, kb, kc]).size).toBe(3);
    act(() => result.current.removeAt(0));
    expect(result.current.items).toEqual(["b", "c"]);
    // "b" is now index 0 but keeps its own key — no identity hand-off.
    expect(result.current.keys).toEqual([kb, kc]);
  });

  it("gives an appended row a fresh key and reconciles an external reset by length", () => {
    const { result } = renderHook(() => useHarness(["a"]));
    const [ka] = result.current.keys;
    act(() => result.current.append("b"));
    expect(result.current.keys[0]).toBe(ka);
    expect(result.current.keys[1]).not.toBe(ka);
    // Somebody else shrinks the list (a reset): keys follow the length.
    act(() => result.current.replaceAt(1, "b2"));
    expect(result.current.keys).toHaveLength(2);
  });
});
