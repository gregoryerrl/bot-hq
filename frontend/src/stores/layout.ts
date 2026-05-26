import { create } from "zustand";

interface LayoutState {
  sidebarCollapsed: boolean;
  setSidebarCollapsed: (collapsed: boolean) => void;
}

export const useLayoutStore = create<LayoutState>((set) => ({
  sidebarCollapsed: false,
  setSidebarCollapsed: (collapsed) => set({ sidebarCollapsed: collapsed }),
}));
