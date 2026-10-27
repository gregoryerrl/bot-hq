import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { composeSnippet, type SnippetName } from "./webviewSnippets";

/**
 * Executes the real `src/signaling/webview/*.js` bytes in jsdom through the
 * same composition the app evaluates (`./webviewSnippets.ts`). What this
 * proves: the guards refuse BEFORE any effect, with the structured reasons the
 * Rust side relays, and an admitted action does what it says. What it does not
 * prove: the live window (a separate smoke), or the Rust admission/lease
 * bookkeeping (Rust tests).
 */
// Indirect eval: global scope, where jsdom's window/document/location live.
const run = (action: SnippetName, args: Record<string, unknown>) => (0, eval)(composeSnippet(action, args));
const go = (path: string) => window.history.pushState({}, "", path);

type Lease = { gen: number; until_ms: number | null };
const lease = (gen: number, until_ms: number | null = null): Lease => ({ gen, until_ms });

beforeEach(() => {
  document.body.innerHTML = "";
  delete (window as unknown as { __bhq_lease_gen?: number }).__bhq_lease_gen;
  go("/");
});
afterEach(() => {
  vi.restoreAllMocks();
});

describe("route and session guards", () => {
  it("matches expected_route on path-segment boundaries and refuses before acting", () => {
    document.body.innerHTML = `<button id="b">go</button>`;
    const clicks = vi.fn();
    document.getElementById("b")!.addEventListener("click", clicks);

    go("/sessions/s10");
    const refused = run("click", { selector: "#b", expected_route: "/sessions/s1" });
    expect(refused).toMatchObject({ outcome: "refused", ok: false, matched: false, reason: "route", route: "/sessions/s10", session_id: "s10" });
    expect(clicks).not.toHaveBeenCalled();

    go("/sessions/s1/docs");
    expect(run("click", { selector: "#b", expected_route: "/sessions/s1" })).toMatchObject({ outcome: "done", ok: true, session_id: "s1" });
    go("/sessions/s1");
    expect(run("click", { selector: "#b", expected_route: "/sessions/s1/" })).toMatchObject({ outcome: "done" });
    expect(run("click", { selector: "#b", expected_route: "/" })).toMatchObject({ outcome: "done" });
    expect(clicks).toHaveBeenCalledTimes(3);
  });

  it("requires the exact /sessions/<id> segment when expected_session_id is given", () => {
    document.body.innerHTML = `<button id="b">go</button>`;
    const clicks = vi.fn();
    document.getElementById("b")!.addEventListener("click", clicks);
    go("/sessions/s-abc/tray");
    expect(run("click", { selector: "#b", expected_session_id: "s-ab" })).toMatchObject({ reason: "session", session_id: "s-abc" });
    expect(clicks).not.toHaveBeenCalled();
    expect(run("click", { selector: "#b", expected_session_id: "s-abc" })).toMatchObject({ outcome: "done" });
    go("/settings");
    expect(run("click", { selector: "#b", expected_session_id: "s-abc" })).toMatchObject({ reason: "session", session_id: null, route: "/settings" });
  });
});

describe("selectors", () => {
  it("refuses a selector that matches nothing — press_key never falls back to the focused element", () => {
    document.body.innerHTML = `<input id="composer" />`;
    const input = document.getElementById("composer") as HTMLInputElement;
    const keys = vi.fn();
    input.addEventListener("keydown", keys);
    input.focus();
    expect(document.activeElement).toBe(input);
    expect(run("press_key", { selector: "#nope", key: "Enter" })).toMatchObject({ outcome: "refused", reason: "no_match", detail: { selector: "#nope" } });
    expect(keys).not.toHaveBeenCalled();
    // Without a selector the active element is the stated target.
    expect(run("press_key", { key: "Enter" })).toMatchObject({ outcome: "done", target: "active_element", target_tag: "input" });
    expect(keys).toHaveBeenCalledTimes(1);
    expect(run("press_key", { selector: "#composer", key: "Escape" })).toMatchObject({ outcome: "done", target: "selector" });
    expect(keys).toHaveBeenCalledTimes(2);
  });

  it("reports an invalid CSS selector instead of throwing", () => {
    expect(run("click", { selector: "[[[" })).toMatchObject({ outcome: "refused", reason: "invalid_selector" });
  });
});

describe("type", () => {
  it("sets an input through the prototype setter and fires input + change", () => {
    document.body.innerHTML = `<textarea id="t"></textarea>`;
    const t = document.getElementById("t") as HTMLTextAreaElement;
    const events: string[] = [];
    t.addEventListener("input", () => events.push("input"));
    t.addEventListener("change", () => events.push("change"));
    expect(run("type", { selector: "#t", text: "hello there" })).toMatchObject({ outcome: "done", length: 11, applied: true });
    expect(t.value).toBe("hello there");
    expect(events).toEqual(["input", "change"]);
    expect(document.activeElement).toBe(t);
  });

  it("picks a select option by value or label, and refuses ambiguity and misses without touching it", () => {
    document.body.innerHTML = `<select id="s"><option value="1">HANDS</option><option value="2">EYES</option><option value="3">EYES</option></select>`;
    const s = document.getElementById("s") as HTMLSelectElement;
    const changes = vi.fn();
    s.addEventListener("change", changes);
    expect(run("type", { selector: "#s", text: "HANDS" })).toMatchObject({ outcome: "done", selected: "1", applied: true });
    expect(s.value).toBe("1");
    expect(run("type", { selector: "#s", text: "2" })).toMatchObject({ outcome: "done", selected: "2", applied: true });
    expect(changes).toHaveBeenCalledTimes(2);
    expect(run("type", { selector: "#s", text: "EYES" })).toMatchObject({ reason: "ambiguous_option" });
    expect(run("type", { selector: "#s", text: "nobody" })).toMatchObject({ reason: "no_option" });
    expect(s.value).toBe("2");
    expect(changes).toHaveBeenCalledTimes(2);
  });

  it("clicks a checkbox only when its state differs, refuses other words, and refuses a non-editable target", () => {
    document.body.innerHTML = `<input id="c" type="checkbox" /><div id="d">text</div>`;
    const c = document.getElementById("c") as HTMLInputElement;
    expect(run("type", { selector: "#c", text: "true" })).toMatchObject({ outcome: "done", checked: true, applied: true });
    expect(run("type", { selector: "#c", text: "on" })).toMatchObject({ outcome: "done", checked: true, applied: true });
    expect(run("type", { selector: "#c", text: "maybe" })).toMatchObject({ reason: "bad_checkbox_value" });
    expect(c.checked).toBe(true);
    expect(run("type", { selector: "#c", text: "false" })).toMatchObject({ outcome: "done", checked: false });
    expect(run("type", { selector: "#d", text: "x" })).toMatchObject({ reason: "not_editable", detail: { tag: "div" } });
  });
});

describe("scroll", () => {
  it("scrolls an element, or the window when no selector is given", () => {
    document.body.innerHTML = `<div id="log" style="height: 50px; overflow: auto"><div style="height: 2000px"></div></div>`;
    const el = document.getElementById("log")!;
    expect(run("scroll", { selector: "#log", y: 120 })).toMatchObject({ outcome: "done" });
    expect(el.scrollTop).toBe(120);
    const scrollTo = vi.fn();
    window.scrollTo = scrollTo as unknown as typeof window.scrollTo;
    expect(run("scroll", { y: 300 })).toMatchObject({ outcome: "done" });
    expect(scrollTo).toHaveBeenCalledWith({ top: 300, behavior: "auto" });
  });
});

describe("the lease, checked at actuation (eyes-2, 03:40Z)", () => {
  it("the mirror is monotonic: a late, lower generation never rolls it back", () => {
    expect(run("mirror", { gen: 2 })).toMatchObject({ gen: 2, applied: true });
    expect(run("mirror", { gen: 1 })).toMatchObject({ gen: 2, applied: false, requested: 1 });
    expect(run("mirror", { gen: 3 })).toMatchObject({ gen: 3, applied: true });
    expect(run("context", {})).toMatchObject({ lease_gen: 3 });
  });

  it("an action admitted under an older generation is refused with no effect once the mirror moved on", () => {
    document.body.innerHTML = `<button id="b">go</button>`;
    const clicks = vi.fn();
    document.getElementById("b")!.addEventListener("click", clicks);
    run("mirror", { gen: 1 });
    expect(run("click", { selector: "#b", lease: lease(1, Date.now() + 60_000) })).toMatchObject({ outcome: "done" });
    // A takeover happened while this action sat in the queue.
    run("mirror", { gen: 2 });
    expect(run("click", { selector: "#b", lease: lease(1, Date.now() + 60_000) })).toMatchObject({ reason: "lease_lost", detail: { admitted_gen: 1, current_gen: 2 } });
    // A non-holder admitted while the lease was free carries the generation it saw.
    expect(run("click", { selector: "#b", lease: lease(2, null) })).toMatchObject({ outcome: "done" });
    expect(run("click", { selector: "#b", lease: lease(1, null) })).toMatchObject({ reason: "lease_lost" });
    expect(clicks).toHaveBeenCalledTimes(2);
  });

  it("a holder's action that runs after its deadline is refused even though the generation never changed", () => {
    document.body.innerHTML = `<input id="t" />`;
    run("mirror", { gen: 1 });
    const t = document.getElementById("t") as HTMLInputElement;
    // Admitted while valid, executed 3 s late with nobody else acquiring.
    const past = Date.now() - 2_000;
    expect(run("type", { selector: "#t", text: "late", lease: lease(1, past) })).toMatchObject({ reason: "lease_expired", detail: { until_ms: past } });
    expect(t.value).toBe("");
    expect(run("type", { selector: "#t", text: "in time", lease: lease(1, Date.now() + 1_000) })).toMatchObject({ outcome: "done" });
    expect(t.value).toBe("in time");
  });

  it("a window that never saw a mirror reads as generation 0", () => {
    document.body.innerHTML = `<button id="b">go</button>`;
    expect(run("click", { selector: "#b", lease: lease(0, null) })).toMatchObject({ outcome: "done" });
    expect(run("click", { selector: "#b", lease: lease(1, null) })).toMatchObject({ reason: "lease_lost", detail: { current_gen: 0 } });
  });

  it("guards run in order: lease, route, session, selector — the first failure is the reason", () => {
    document.body.innerHTML = ``;
    run("mirror", { gen: 5 });
    go("/settings");
    expect(run("click", { selector: "#none", expected_route: "/sessions/s1", lease: lease(4, null) })).toMatchObject({ reason: "lease_lost" });
    expect(run("click", { selector: "#none", expected_route: "/sessions/s1", lease: lease(5, null) })).toMatchObject({ reason: "route" });
    expect(run("click", { selector: "#none", expected_session_id: "s1", lease: lease(5, null) })).toMatchObject({ reason: "session" });
    expect(run("click", { selector: "#none", lease: lease(5, null) })).toMatchObject({ reason: "no_match" });
  });
});

describe("truthfulness about the DOM (eyes-2, 5d1b4102 / 2e17092b)", () => {
  it("an exception anywhere in the snippet is an unknown outcome, never a bare value", () => {
    document.body.innerHTML = `<button id="b">go</button>`;
    // Rust validates argument types first; the composition's catch is what a
    // window that throws for any other reason falls back on.
    const reply = run("click", { selector: "#b", expected_route: 42 as unknown as string });
    expect(reply).toMatchObject({ outcome: "unknown", ok: false, matched: null, reason: "exception" });
    expect(typeof reply.detail).toBe("string");
  });

  it("refuses a disabled or non-clickable target before clicking", () => {
    document.body.innerHTML = `<button id="d" disabled>no</button><fieldset disabled><button id="f">no</button></fieldset><svg id="s"></svg>`;
    const clicks = vi.fn();
    document.getElementById("d")!.addEventListener("click", clicks);
    expect(run("click", { selector: "#d" })).toMatchObject({ outcome: "refused", reason: "disabled" });
    expect(run("click", { selector: "#f" })).toMatchObject({ outcome: "refused", reason: "disabled" });
    expect(run("click", { selector: "#s" })).toMatchObject({ outcome: "refused", reason: "not_clickable", detail: { tag: "svg" } });
    expect(clicks).not.toHaveBeenCalled();
  });

  it("refuses disabled, readonly and file inputs before focusing them", () => {
    document.body.innerHTML = `<input id="d" disabled /><input id="r" readonly value="keep" /><input id="f" type="file" /><input id="c" type="checkbox" disabled />`;
    expect(run("type", { selector: "#d", text: "x" })).toMatchObject({ reason: "disabled" });
    expect(run("type", { selector: "#r", text: "x" })).toMatchObject({ reason: "readonly" });
    expect(run("type", { selector: "#f", text: "x" })).toMatchObject({ reason: "not_editable", detail: { type: "file" } });
    expect(run("type", { selector: "#c", text: "true" })).toMatchObject({ reason: "disabled" });
    expect((document.getElementById("r") as HTMLInputElement).value).toBe("keep");
    expect((document.getElementById("c") as HTMLInputElement).checked).toBe(false);
    expect(document.activeElement).toBe(document.body);
  });

  it("reports the observed value, not the requested one, when the browser sanitizes it", () => {
    document.body.innerHTML = `<input id="n" type="number" /><input id="t" />`;
    const sanitized = run("type", { selector: "#n", text: "abc" });
    expect(sanitized).toMatchObject({ outcome: "done", applied: false, length: 0 });
    expect(run("type", { selector: "#t", text: "abc" })).toMatchObject({ outcome: "done", applied: true, length: 3 });
  });
});

describe("context", () => {
  it("reads the route, the session segment and the mirrored generation without effects", () => {
    go("/sessions/s-xyz/docs");
    expect(run("context", {})).toEqual({ outcome: "done", ok: true, matched: true, route: "/sessions/s-xyz/docs", session_id: "s-xyz", lease_gen: 0 });
  });
});
