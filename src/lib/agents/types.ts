export interface AgentMessage {
  type: "assistant" | "user" | "system" | "result";
  content: string;
  timestamp: Date;
}

export interface AgentOutput {
  type: "text" | "tool_use" | "tool_result" | "error";
  content: string;
  toolName?: string;
  toolInput?: Record<string, unknown>;
}

export interface AgentSession {
  id: number;
  workspaceId: number;
  taskId: number | null;
  pid: number | null;
  status: "running" | "idle" | "stopped" | "error";
  messages: AgentMessage[];
}

export type AgentEventHandler = (event: {
  type: "output" | "error" | "exit" | "approval_needed";
  data: unknown;
}) => void;
