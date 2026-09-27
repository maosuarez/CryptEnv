import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import type { VaultItem, Category, Screen, MenuState, GlobalToggleResult, ItemOwner } from '../types';
import { t } from '../i18n';

/** The backend's error string for any key-requiring call made while locked. */
export function isVaultLockedError(e: unknown): boolean {
  return String(e).includes('vault is locked');
}

export const CAT_COLORS_PRESET = [
  '#FF9900', '#10a37f', '#635bff', '#c9d1d9',
  'oklch(0.62 0.16 280)', 'oklch(0.70 0.17 162)',
  'oklch(0.70 0.17 220)', 'oklch(0.62 0.20 22)',
];

interface ToastState {
  msg: string;
  type: 'success' | 'error';
}

/** Max remembered screens for `goBack()`; older entries are dropped. */
export const HISTORY_LIMIT = 20;

/** Resolves the screen `goBack()` lands on: the most recent history entry that
 *  differs from `current`, or the `projects` landing page when none remains. */
export function popHistory(history: Screen[], current: Screen): { screen: Screen; history: Screen[] } {
  const rest = [...history];
  while (rest.length > 0) {
    const prev = rest.pop()!;
    if (prev !== current && prev !== 'lock') return { screen: prev, history: rest };
  }
  return { screen: 'projects', history: [] };
}

interface VaultStore {
  screen:      Screen;
  /** Screens visited before `screen`, most recent last. Cleared on lock/unlock. */
  history:     Screen[];
  items:       VaultItem[];
  cats:        Category[];
  editTarget:  VaultItem | null;
  menu:        MenuState | null;
  toast:       ToastState | null;
  placeholder: VaultItem | null;
  lockTimeout: number;   // minutes; 0 = never
  hotkey:      string;

  go:             (screen: Screen) => void;
  goBack:         () => void;
  setEditTarget:  (item: VaultItem | null) => void;
  openMenu:       (menu: MenuState) => void;
  closeMenu:      () => void;
  showToast:      (msg: string, type?: 'success' | 'error') => void;
  setPlaceholder: (item: VaultItem | null) => void;
  setLockTimeout: (mins: number) => void;
  setHotkey:      (key: string) => void;
  unlock:              (password: string) => Promise<void>;
  unlockWithPayload:   (payload: { items: VaultItem[]; categories: Category[] }) => Promise<void>;
  lock:                () => Promise<void>;
  /** The backend already locked (auto-lock event or a "vault is locked"
   *  error): drop in-memory vault state and show the lock screen. */
  lockedByBackend:     () => void;
  wipe:           () => Promise<void>;
  saveItem:       (form: Omit<VaultItem, 'id' | 'created'>) => Promise<void>;
  deleteItem:     (id: number) => Promise<void>;
  saveCats:       (cats: Category[]) => Promise<void>;
  toggleGlobal:   (id: number, global: boolean) => Promise<GlobalToggleResult>;
  getItemOwners:  (id: number) => Promise<ItemOwner[]>;
}

let toastTimer: ReturnType<typeof setTimeout>;

export const useVaultStore = create<VaultStore>((set, get) => ({
  screen:      'lock',
  history:     [],
  items:       [],
  cats:        [],
  editTarget:  null,
  menu:        null,
  toast:       null,
  placeholder: null,
  lockTimeout: 5,
  hotkey:      'Ctrl+Alt+Z',

  go: (screen) => set((s) => {
    if (screen === s.screen) return {};
    // The lock screen is never a back target; adjacent duplicates are collapsed.
    const push = s.screen !== 'lock' && s.history[s.history.length - 1] !== s.screen;
    const history = push ? [...s.history, s.screen].slice(-HISTORY_LIMIT) : s.history;
    return { screen, history };
  }),

  goBack: () => set((s) => popHistory(s.history, s.screen)),

  setEditTarget: (editTarget) => set({ editTarget }),

  openMenu: (menu) => set({ menu }),

  closeMenu: () => set({ menu: null }),

  showToast: (msg, type = 'success') => {
    set({ toast: { msg, type } });
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => set({ toast: null }), 2200);
  },

  setPlaceholder: (placeholder) => set({ placeholder }),

  setLockTimeout: (lockTimeout) => set({ lockTimeout }),

  setHotkey: (hotkey) => set({ hotkey }),

  unlock: async (password) => {
    const [result, settings] = await Promise.all([
      invoke<{ items: VaultItem[]; categories: Category[] }>('vault_unlock', { password }),
      invoke<{ autoLockTimeout: number; hotkey: string }>('vault_get_settings'),
    ]);
    set({
      items:       result.items,
      cats:        result.categories,
      screen:      'projects',
      history:     [],
      editTarget:  null,
      lockTimeout: settings.autoLockTimeout,
      hotkey:      settings.hotkey,
    });
  },

  unlockWithPayload: async (payload) => {
    const settings = await invoke<{ autoLockTimeout: number; hotkey: string }>('vault_get_settings');
    set({
      items:       payload.items,
      cats:        payload.categories,
      screen:      'projects',
      history:     [],
      editTarget:  null,
      lockTimeout: settings.autoLockTimeout,
      hotkey:      settings.hotkey,
    });
  },

  lock: async () => {
    try {
      await invoke('vault_lock');
    } catch {}
    set({ screen: 'lock', history: [], items: [], cats: [], editTarget: null, menu: null });
  },

  lockedByBackend: () => {
    if (get().screen === 'lock') return;
    set({ screen: 'lock', history: [], items: [], cats: [], editTarget: null, menu: null });
    get().showToast(t('lock.sessionLocked'), 'error');
  },

  wipe: async () => {
    await invoke('vault_wipe');
    set({ screen: 'lock', history: [], items: [], cats: [], editTarget: null, menu: null });
  },

  saveItem: async (form) => {
    const { editTarget } = get();
    const itemToSave: VaultItem = {
      ...form,
      id:      editTarget?.id ?? 0,
      created: editTarget?.created ?? new Date().toISOString().slice(0, 10),
    } as VaultItem;

    const saved = await invoke<VaultItem>('vault_save_item', { item: itemToSave });

    set((s) => ({
      items: editTarget
        ? s.items.map((i) => (i.id === saved.id ? saved : i))
        : [...s.items, saved],
      editTarget: null,
      // Return to whichever screen opened the editor.
      ...popHistory(s.history, s.screen),
    }));
  },

  deleteItem: async (id) => {
    await invoke('vault_delete_item', { id });
    set((s) => ({ items: s.items.filter((i) => i.id !== id), screen: 'vault' }));
  },

  saveCats: async (cats) => {
    await invoke('vault_save_categories', { cats });
    set({ cats });
  },

  toggleGlobal: async (id, global) => {
    const result = await invoke<GlobalToggleResult>('vault_set_item_global', { id, global });
    set((s) => {
      if (result.updated) {
        const updated = result.updated;
        return { items: s.items.map((i) => (i.id === id ? updated : i)) };
      }
      // Un-globaling a multi-owner item forks it — the shared row is gone,
      // replaced by one independent copy per project that owned it.
      return { items: s.items.filter((i) => i.id !== id).concat(result.forked) };
    });
    return result;
  },

  getItemOwners: (id) => invoke<ItemOwner[]>('vault_get_item_owners', { id }),
}));
