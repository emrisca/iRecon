import { createContext, useContext } from "react";

export type SidebarState = {
  open: boolean;
  toggle: () => void;
  live: boolean;
  liveBusy: boolean;
  liveProject: string | null;
  onToggleLive: () => void;
};

export const SidebarContext = createContext<SidebarState>({
  open: true,
  toggle: () => {},
  live: false,
  liveBusy: false,
  liveProject: null,
  onToggleLive: () => {},
});

export function useSidebar() {
  return useContext(SidebarContext);
}
