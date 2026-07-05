import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { CspConsentSection } from "./PluginManager";
import type { CspExtraOrigins } from "../lib/bindings";

describe("CspConsentSection", () => {
  it("lists the exact origins per directive, scheme stripped", () => {
    const csp: CspExtraOrigins = {
      "script-src": ["https://cdn.jsdelivr.net", "https://unpkg.com"],
      "style-src": ["https://fonts.googleapis.com"],
      "font-src": ["https://fonts.gstatic.com"],
      "img-src": [],
    };
    render(<CspConsentSection csp={csp} />);
    expect(
      screen.getByText(/Can load and run code from: cdn\.jsdelivr\.net, unpkg\.com/),
    ).toBeTruthy();
    expect(
      screen.getByText(/Can load styles from: fonts\.googleapis\.com/),
    ).toBeTruthy();
    expect(
      screen.getByText(/Can load fonts from: fonts\.gstatic\.com/),
    ).toBeTruthy();
    // Empty directive renders no line.
    expect(screen.queryByText(/Can load images from/)).toBeNull();
  });

  it("tolerates omitted directives (wire format skips empty vecs)", () => {
    // The generated type claims all four keys exist; the wire format omits
    // empty ones. The component must not crash on a partial object.
    const partial = {
      "script-src": ["https://cdn.jsdelivr.net"],
    } as unknown as CspExtraOrigins;
    render(<CspConsentSection csp={partial} />);
    expect(
      screen.getByText(/Can load and run code from: cdn\.jsdelivr\.net/),
    ).toBeTruthy();
  });

  it("renders nothing when every directive is empty", () => {
    const empty = {} as unknown as CspExtraOrigins;
    const { container } = render(<CspConsentSection csp={empty} />);
    expect(container.innerHTML).toBe("");
  });
});
