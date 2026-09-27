import { create } from 'zustand';
import type { WslActionReport } from '../types';

export interface WslLastAction {
  distro: string;
  action: 'configure' | 'remove';
  report: WslActionReport;
}

interface WslStore {
  /** Distro the user last acted on — highlights its row and scopes the report. */
  selectedDistro: string | null;
  lastAction:     WslLastAction | null;

  selectDistro:  (distro: string | null) => void;
  setLastAction: (action: WslLastAction | null) => void;
}

export const useWslStore = create<WslStore>((set) => ({
  selectedDistro: null,
  lastAction:     null,

  selectDistro:  (selectedDistro) => set({ selectedDistro }),
  setLastAction: (lastAction) => set({ lastAction }),
}));
