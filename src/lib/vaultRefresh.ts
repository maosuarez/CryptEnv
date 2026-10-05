import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import type { QueryClient } from '@tanstack/react-query';
import { isVaultLockedError, useVaultStore } from '../store';
import { useProjectStore } from '../store/projectStore';
import type { Category, VaultItem } from '../types';

/** Quiet period that collapses a burst of `vault_changed` events (e.g. a
 *  `crypt-env add` of a whole .env file) into one refetch. */
export const REFRESH_DEBOUNCE_MS = 100;

/** Re-reads everything the window shows — items, categories, projects and
 *  environments, plus any TanStack Query cache — from the backend. A no-op
 *  on the lock screen (nothing is loaded there). Errors never carry secrets;
 *  a locked vault sends the UI back to the lock screen. */
export async function refreshVault(queryClient?: QueryClient): Promise<void> {
  if (useVaultStore.getState().screen === 'lock') return;
  try {
    const [list] = await Promise.all([
      invoke<{ items: VaultItem[]; categories: Category[] }>('vault_list'),
      useProjectStore.getState().load(),
    ]);
    useVaultStore.setState({ items: list.items, cats: list.categories });
    await queryClient?.invalidateQueries();
  } catch (e) {
    if (isVaultLockedError(e)) {
      useVaultStore.getState().lockedByBackend();
    } else {
      console.error('Refresh failed:', e);
    }
  }
}

/** True while a user-triggered refresh (button or shortcut) is running. */
export const useRefreshing = create<{ refreshing: boolean }>(() => ({ refreshing: false }));

/** Refresh the user asked for: same as `refreshVault`, plus the busy flag the
 *  titlebar button animates. */
export async function manualRefresh(queryClient?: QueryClient): Promise<void> {
  if (useRefreshing.getState().refreshing) return;
  useRefreshing.setState({ refreshing: true });
  try {
    await refreshVault(queryClient);
  } finally {
    useRefreshing.setState({ refreshing: false });
  }
}

/** Trailing-edge debounce: `fn` runs once, `ms` after the last call. */
export function debounce(fn: () => void, ms: number): { call: () => void; cancel: () => void } {
  let timer: ReturnType<typeof setTimeout> | undefined;
  return {
    call: () => {
      clearTimeout(timer);
      timer = setTimeout(fn, ms);
    },
    cancel: () => clearTimeout(timer),
  };
}

/** F5 / Ctrl+R / Cmd+R — the browser reload chords the webview would
 *  otherwise act on. */
export function isRefreshShortcut(e: Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey'>): boolean {
  if (e.key === 'F5') return true;
  if (e.altKey || e.shiftKey) return false;
  return (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'r';
}
