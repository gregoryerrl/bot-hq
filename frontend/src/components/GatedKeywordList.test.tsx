import { describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@testing-library/react";
import { GatedKeywordList } from "./GatedKeywordList";
import type { GatedKeyword } from "../lib/bindings";

const rows = (modes: GatedKeyword["mode"][]): GatedKeyword[] =>
  modes.map((mode, i) => ({ keyword: `kw-${i}`, mode }));

const renderList = (value: GatedKeyword[], onChange = vi.fn()) => {
  const utils = render(
    <GatedKeywordList
      value={value}
      onChange={onChange}
      emptyState={<span>empty</span>}
      footer={() => null}
    />,
  );
  return { ...utils, onChange };
};

describe("GatedKeywordList — Set all (2026-08-27)", () => {
  it("maps every row's mode in ONE emit — the one-by-one friction it replaces", () => {
    const { getAllByText, onChange } = renderList(
      rows(["gate", "auto_allow", "gate", "gate"]),
    );
    // Set-all renders above the list, so its button is the first match; the
    // per-row SegToggles carry the same label.
    fireEvent.click(getAllByText("Auto-allow", { selector: "button" })[0]);
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange.mock.calls[0][0]).toEqual(
      rows(["auto_allow", "auto_allow", "auto_allow", "auto_allow"]),
    );
  });

  it("sets all back to gate, keywords untouched", () => {
    const { getAllByText, onChange } = renderList(rows(["auto_allow", "auto_allow"]));
    // Two "Gate" texts exist per row toggle too; the Set-all one is the first button.
    fireEvent.click(getAllByText("Gate", { selector: "button" })[0]);
    expect(onChange.mock.calls[0][0]).toEqual(rows(["gate", "gate"]));
  });

  it("does not render for a single row — nothing to bulk-set", () => {
    const { queryByText } = renderList(rows(["gate"]));
    expect(queryByText("Set all")).toBeNull();
  });
});
