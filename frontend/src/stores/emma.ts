import { create } from "zustand";

interface EmmaState {
  open: boolean;
  setOpen: (open: boolean) => void;
  toggle: () => void;
}

export const useEmmaStore = create<EmmaState>((set) => ({
  open: false,
  setOpen: (open) => set({ open }),
  toggle: () => set((s) => ({ open: !s.open })),
}));
