import { createContext, useContext } from "react";

export type SidebarState = {
  open: boolean;
  toggle: () => void;
  live: boolean;
  liveBusy: boolean;
  onToggleLive: () => void;
};

export const SidebarContext = createContext<SidebarState>({
  open: true,
  toggle: () => {},
  live: false,
  liveBusy: false,
  onToggleLive: () => {},
});

export function useSidebar() {
  return useContext(SidebarContext);
}
