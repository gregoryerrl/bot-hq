import { fireEvent, render } from "@testing-library/react";
import { createElement, Fragment } from "react";
import { useEscapeKey } from "./useEscapeKey";

// A .ts file (the codebase map pairs a test with its subject by extension),
// so the overlays are built with createElement rather than JSX.

function Overlay({ onClose, enabled = true }: { onClose: () => void; enabled?: boolean }) {
  useEscapeKey(onClose, enabled);
  return null;
}
const overlay = (onClose: () => void, enabled = true, key?: string) => createElement(Overlay, { onClose, enabled, key });
const two = (a: ReturnType<typeof overlay>, b: ReturnType<typeof overlay>) => createElement(Fragment, null, a, b);

describe("useEscapeKey", () => {
  it("answers only the newest open overlay, then the one beneath it", () => {
    const outer = vi.fn();
    const inner = vi.fn();
    const { rerender } = render(two(overlay(outer, true, "a"), overlay(inner, true, "b")));
    fireEvent.keyDown(window, { key: "Escape" });
    expect(inner).toHaveBeenCalledTimes(1);
    expect(outer).not.toHaveBeenCalled();
    // The inner one closes (stops listening): the next press reaches the outer.
    rerender(two(overlay(outer, true, "a"), overlay(inner, false, "b")));
    fireEvent.keyDown(window, { key: "Escape" });
    expect(outer).toHaveBeenCalledTimes(1);
    expect(inner).toHaveBeenCalledTimes(1);
  });

  it("calls the latest handler without losing its place", () => {
    const first = vi.fn();
    const second = vi.fn();
    const top = vi.fn();
    const { rerender } = render(two(overlay(first, true, "a"), overlay(top, true, "b")));
    // The bottom overlay re-renders with a new closure; it must not jump on top.
    rerender(two(overlay(second, true, "a"), overlay(top, true, "b")));
    fireEvent.keyDown(window, { key: "Escape" });
    expect(top).toHaveBeenCalledTimes(1);
    expect(first).not.toHaveBeenCalled();
    expect(second).not.toHaveBeenCalled();
  });

  it("ignores other keys and does nothing with no overlay open", () => {
    const onClose = vi.fn();
    const { unmount } = render(overlay(onClose));
    fireEvent.keyDown(window, { key: "Enter" });
    expect(onClose).not.toHaveBeenCalled();
    unmount();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).not.toHaveBeenCalled();
  });
});
