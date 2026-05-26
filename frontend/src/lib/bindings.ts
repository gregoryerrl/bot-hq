// Hand-stubbed types matching the tauri-specta export. The Rust main.rs
// auto-overwrites this file at app startup via
// `tauri_specta_gen::builder().export(..., "frontend/src/lib/bindings.ts")`,
// so in dev with the Tauri binary running these stubs are replaced. The
// committed version is the source-of-truth for `tsc --noEmit` + `npm run
// build` in CI/sandbox environments where the Rust app isn't running.

export type AppError =
  | { kind: "Validation"; message: string }
  | { kind: "NotFound"; message: string }
  | { kind: "Unauthorized"; message: string }
  | { kind: "Internal"; message: string }
  | { kind: "DbError"; message: string }
  | { kind: "CapabilityDenied"; message: string };

export interface SessionInfo {
  id: string;
  title: string;
  working_repo_path: string | null;
  archived: boolean;
  created_at: string;
  closed_at: string | null;
  brian_model_at_spawn: string | null;
  rain_model_at_spawn: string | null;
}

export interface AgentMessage {
  id: number;
  session_id: string;
  author: string;
  kind: string;
  content: string;
  created_at: string;
}

export interface AgentConfigView {
  agent_name: string;
  provider: string;
  model_name: string;
  base_url: string | null;
  auth_token: string | null;
  updated_at: string;
}

export interface ClIndexEntryView {
  id: number;
  project_id: string;
  file_path: string;
  description: string;
  tags: string | null;
  created_at: string;
  updated_at: string;
}

export interface ClFolderView {
  id: number;
  project_id: string;
  folder_path: string;
  description: string;
  tags: string | null;
  created_at: string;
  updated_at: string;
}

export interface ClRescanReportView {
  added: string[];
  touched: string[];
  orphaned: string[];
}

export type GrantScopeView =
  | { kind: "none" }
  | { kind: "all_branches" }
  | { kind: "specific"; branches: string[] };

export type PermissionActionView = "commit" | "push";

export interface SessionPermissionsView {
  commit: GrantScopeView;
  push: GrantScopeView;
}

export interface PendingChoiceView {
  choice_id: string;
  session_id: string;
  agent: string;
  question: string;
  options: string[];
}

export interface SessionDocumentView {
  id: number;
  session_id: string;
  slug: string;
  body: string;
  created_at: string;
  updated_at: string;
  phase: string | null;
}

export interface AwaitingUser {
  session_id: string;
  agent: string;
  reason: string;
}

export interface PhaseChangedEvent {
  session_id: string;
  agent: string;
  target: string;
}
