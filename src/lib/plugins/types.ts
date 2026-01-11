// src/lib/plugins/types.ts

export interface PluginManifest {
  name: string;
  version: string;
  description: string;
  author?: {
    name: string;
    url?: string;
  };
  repository?: string;
  license?: string;

  "bot-hq"?: {
    minVersion?: string;
  };

  mcp?: {
    entry: string;
    transport: "stdio";
    tools?: string[];
  };

  extensions?: string;

  ui?: {
    tabs?: PluginTabDefinition[];
    workspaceSettings?: string;
    taskBadge?: string;
    taskActions?: string;
  };

  settings?: Record<string, PluginSettingDefinition>;
  credentials?: Record<string, PluginCredentialDefinition>;
  permissions?: string[];
}

export interface PluginTabDefinition {
  id: string;
  label: string;
  icon: string;
  component: string;
}

export interface PluginSettingDefinition {
  type: "string" | "number" | "boolean" | "select";
  label: string;
  description?: string;
  default?: string | number | boolean;
  options?: string[]; // For select type
}

export interface PluginCredentialDefinition {
  type: "secret";
  label: string;
  description?: string;
  required?: boolean;
}

export interface LoadedPlugin {
  name: string;
  version: string;
  path: string;
  manifest: PluginManifest;
  enabled: boolean;
  dbId?: number;
}

export interface PluginContext {
  mcp: {
    call: (tool: string, params: Record<string, unknown>) => Promise<unknown>;
  };
  store: {
    get: (key: string) => Promise<unknown>;
    set: (key: string, value: unknown) => Promise<void>;
    delete: (key: string) => Promise<void>;
  };
  workspaceData: {
    get: (workspaceId: number) => Promise<unknown>;
    set: (workspaceId: number, data: unknown) => Promise<void>;
  };
  taskData: {
    get: (taskId: number) => Promise<unknown>;
    set: (taskId: number, data: unknown) => Promise<void>;
  };
  settings: Record<string, unknown>;
  credentials: Record<string, string>;
  log: {
    info: (msg: string) => void;
    warn: (msg: string) => void;
    error: (msg: string) => void;
  };
}

export interface PluginAction {
  id: string;
  label: string;
  description?: string | ((context: ActionContext) => string);
  icon?: string;
  defaultChecked?: boolean;
  handler: (context: ActionContext) => Promise<ActionResult>;
}

export interface ActionContext {
  approval?: {
    id: number;
    branchName: string;
    baseBranch: string;
    commitMessages: string[];
  };
  task?: {
    id: number;
    title: string;
    description: string;
  };
  workspace?: {
    id: number;
    name: string;
    repoPath: string;
  };
  pluginContext: PluginContext;
}

export interface ActionResult {
  success: boolean;
  message?: string;
  error?: string;
  data?: unknown;
}

export interface PluginHooks {
  onTaskCreated?: (task: TaskHookData) => Promise<void>;
  onTaskUpdated?: (task: TaskHookData, changes: Partial<TaskHookData>) => Promise<void>;
  onAgentStart?: (agent: AgentHookData, task: TaskHookData) => Promise<{ context?: string } | void>;
  onAgentComplete?: (agent: AgentHookData, task: TaskHookData) => Promise<void>;
  onApprovalCreated?: (approval: ApprovalHookData) => Promise<void>;
  onApprovalAccepted?: (approval: ApprovalHookData, task: TaskHookData) => Promise<void>;
  onApprovalRejected?: (approval: ApprovalHookData, task: TaskHookData) => Promise<void>;
}

export interface PluginExtensions {
  actions?: {
    approval?: PluginAction[];
    task?: PluginAction[];
    workspace?: PluginAction[];
  };
  hooks?: PluginHooks;
}

export interface TaskHookData {
  id: number;
  workspaceId: number;
  title: string;
  description: string;
  state: string;
  priority: number;
  branchName?: string;
}

export interface AgentHookData {
  sessionId: number;
  workspaceId: number;
  taskId: number;
  status: string;
}

export interface ApprovalHookData {
  id: number;
  taskId: number;
  workspaceId: number;
  branchName: string;
  baseBranch: string;
  commitMessages: string[];
  status: string;
}
