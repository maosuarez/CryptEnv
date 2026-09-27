import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';

interface UpdateStore {
  /** True once the automatic post-unlock check has been started this launch. */
  checked:    boolean;
  /** Newer version reported by the updater, or null. */
  version:    string | null;
  /** User chose LATER — hidden until the app is relaunched. */
  dismissed:  boolean;
  installing: boolean;
  installed:  boolean;
  error:      string | null;

  checkOnce: () => Promise<void>;
  install:   () => Promise<void>;
  dismiss:   () => void;
}

export const useUpdateStore = create<UpdateStore>((set, get) => ({
  checked:    false,
  version:    null,
  dismissed:  false,
  installing: false,
  installed:  false,
  error:      null,

  checkOnce: async () => {
    if (get().checked) return;
    set({ checked: true });
    try {
      const version = await invoke<string | null>('check_for_update');
      if (version) set({ version });
    } catch {
      // Offline or endpoint unreachable — stay silent; Settings still offers a manual check.
    }
  },

  install: async () => {
    set({ installing: true, error: null });
    try {
      await invoke('install_update');
      set({ installing: false, installed: true });
    } catch (e) {
      set({ installing: false, error: String(e) });
    }
  },

  dismiss: () => set({ dismissed: true }),
}));
