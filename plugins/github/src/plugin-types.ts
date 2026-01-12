// Types for plugin extensions - mirrors @bot-hq/plugin-sdk

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

export interface TaskHookData {
  id: number;
  workspaceId: number;
  title: string;
  description: string;
  state: string;
  priority: number;
  branchName?: string;
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

export interface PluginHooks {
  onTaskCreated?: (task: TaskHookData) => Promise<void>;
  onTaskUpdated?: (task: TaskHookData, changes: Partial<TaskHookData>) => Promise<void>;
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
