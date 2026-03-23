"use client";

import { createContext, useContext, useState, useCallback, ReactNode } from "react";

interface CommandContextValue {
  projectId?: number;
  diagramId?: number;
  taskId?: number;
  label?: string;
}

interface CommandContextType {
  context: CommandContextValue;
  setContext: (ctx: CommandContextValue) => void;
  clearContext: () => void;
}

const CommandContext = createContext<CommandContextType>({
  context: {},
  setContext: () => {},
  clearContext: () => {},
});

export function useCommandContext() {
  return useContext(CommandContext);
}

export function CommandContextProvider({ children }: { children: ReactNode }) {
  const [context, setContextState] = useState<CommandContextValue>({});

  const setContext = useCallback((ctx: CommandContextValue) => {
    setContextState(ctx);
  }, []);

  const clearContext = useCallback(() => {
    setContextState({});
  }, []);

  return (
    <CommandContext.Provider value={{ context, setContext, clearContext }}>
      {children}
    </CommandContext.Provider>
  );
}
