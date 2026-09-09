import { useQuery, useMutation, type UseQueryOptions } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import type { AppError } from "../lib/bindings";

/**
 * Query-style Tauri invoke wrapper. Pass the command name + args; TanStack
 * Query handles caching, refetching, loading states. Errors come back typed.
 */
export function useTauriQuery<T>(
  command: string,
  args: Record<string, unknown> = {},
  options?: Omit<UseQueryOptions<T, AppError>, "queryKey" | "queryFn">,
) {
  return useQuery<T, AppError>({
    queryKey: [command, args],
    queryFn: async () => {
      try {
        return await invoke<T>(command, args);
      } catch (err) {
        throw mapError(err);
      }
    },
    ...options,
  });
}

/**
 * Mutation-style Tauri invoke wrapper. Use for create/update/delete commands
 * that aren't safe to retry implicitly.
 */
export function useTauriMutation<TResult, TArgs extends Record<string, unknown>>(
  command: string,
) {
  return useMutation<TResult, AppError, TArgs>({
    mutationFn: async (args) => {
      try {
        return await invoke<TResult>(command, args);
      } catch (err) {
        throw mapError(err);
      }
    },
  });
}

function mapError(err: unknown): AppError {
  if (err && typeof err === "object" && "kind" in err) {
    return err as AppError;
  }
  return { kind: "Internal", message: String(err) };
}

/**
 * Extract a human-readable string from an unknown error — raw `invoke`
 * rejections, thrown values, or AppError objects. For call sites that invoke()
 * directly (fire-and-forget mutations) instead of going through the hooks.
 */
export function errorMessage(err: unknown): string {
  return err && typeof err === "object" && "message" in err
    ? String((err as { message: unknown }).message)
    : String(err);
}
