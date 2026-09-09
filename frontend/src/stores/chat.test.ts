import { describe, it, expect, beforeEach } from "vitest";
import { useChatStore } from "./chat";
import type { AgentMessage } from "../lib/bindings";

function msg(id: number, session: string, content: string): AgentMessage {
  return {
    id,
    session_id: session,
    author: "brian",
    kind: "text",
    content,
    created_at: "2026-05-26T18:00:00Z",
  };
}

describe("chat store", () => {
  beforeEach(() => {
    useChatStore.setState({ messages: {}, watermarks: {}, resolvedToolIds: {} });
  });

  it("setMessages replaces the list and sets watermark to last id", () => {
    useChatStore.getState().setMessages("s1", [msg(1, "s1", "a"), msg(3, "s1", "b")]);
    expect(useChatStore.getState().messages.s1).toHaveLength(2);
    expect(useChatStore.getState().watermarks.s1).toBe(3);
  });

  it("applyBatch appends and advances watermark", () => {
    useChatStore.getState().setMessages("s1", [msg(1, "s1", "a")]);
    useChatStore.getState().applyBatch([msg(2, "s1", "b"), msg(3, "s1", "c")]);
    expect(useChatStore.getState().messages.s1).toHaveLength(3);
    expect(useChatStore.getState().watermarks.s1).toBe(3);
  });

  it("applyBatch dedupes messages with id <= watermark", () => {
    useChatStore.getState().setMessages("s1", [msg(1, "s1", "a"), msg(2, "s1", "b")]);
    // Duplicate id 2 + new id 3
    useChatStore.getState().applyBatch([msg(2, "s1", "dup"), msg(3, "s1", "c")]);
    expect(useChatStore.getState().messages.s1).toHaveLength(3);
    expect(useChatStore.getState().messages.s1[2].content).toBe("c");
  });

  it("clear drops a session", () => {
    useChatStore.getState().setMessages("s1", [msg(1, "s1", "a")]);
    useChatStore.getState().clear("s1");
    expect(useChatStore.getState().messages.s1).toBeUndefined();
  });

  // Round 9: the resolved-tool-id set is kept in the store, incrementally,
  // instead of re-derived (and JSON.parsed) from the whole list per batch.
  function toolUse(id: number, session: string, toolUseId: string): AgentMessage {
    return { ...msg(id, session, JSON.stringify({ id: toolUseId, name: "Bash" })), kind: "tool_use" };
  }
  function toolResult(id: number, session: string, toolUseId: string): AgentMessage {
    return { ...msg(id, session, JSON.stringify({ tool_use_id: toolUseId })), kind: "tool_result" };
  }

  it("resolvedToolIds is seeded by setMessages and grows with applyBatch and prependOlder", () => {
    const st = useChatStore.getState();
    st.setMessages("s1", [toolUse(10, "s1", "t-a"), toolResult(11, "s1", "t-a"), toolUse(12, "s1", "t-b")]);
    expect([...useChatStore.getState().resolvedToolIds.s1]).toEqual(["t-a"]);
    useChatStore.getState().applyBatch([toolResult(13, "s1", "t-b")]);
    expect(useChatStore.getState().resolvedToolIds.s1.has("t-b")).toBe(true);
    // An older page can carry results for older calls.
    useChatStore.getState().prependOlder("s1", [toolResult(5, "s1", "t-old")]);
    expect(useChatStore.getState().resolvedToolIds.s1.has("t-old")).toBe(true);
    // Sessions are isolated; clear drops the set with the messages.
    expect(useChatStore.getState().resolvedToolIds.s2).toBeUndefined();
    useChatStore.getState().clear("s1");
    expect(useChatStore.getState().resolvedToolIds.s1).toBeUndefined();
  });

  it("resolvedToolIds keeps its identity when a batch adds no result", () => {
    useChatStore.getState().setMessages("s1", [toolResult(1, "s1", "t-a")]);
    const before = useChatStore.getState().resolvedToolIds.s1;
    useChatStore.getState().applyBatch([msg(2, "s1", "prose"), toolResult(3, "s1", "t-a")]);
    expect(useChatStore.getState().resolvedToolIds.s1).toBe(before);
    // Unparseable result rows are skipped, not fatal.
    useChatStore.getState().applyBatch([{ ...msg(4, "s1", "not json"), kind: "tool_result" }]);
    expect(useChatStore.getState().resolvedToolIds.s1).toBe(before);
  });
});
