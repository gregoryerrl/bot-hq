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
    useChatStore.setState({ messages: {}, watermarks: {} });
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
});
