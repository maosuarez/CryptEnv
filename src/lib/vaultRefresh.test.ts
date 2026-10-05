import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

import { useVaultStore } from '../store';
import { useProjectStore } from '../store/projectStore';
import { debounce, isRefreshShortcut, manualRefresh, refreshVault } from './vaultRefresh';

const key = (k: string, mods: Partial<KeyboardEvent> = {}) =>
  ({ key: k, ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...mods }) as KeyboardEvent;

describe('isRefreshShortcut', () => {
  it('matches F5, Ctrl+R and Cmd+R only', () => {
    expect(isRefreshShortcut(key('F5'))).toBe(true);
    expect(isRefreshShortcut(key('r', { ctrlKey: true }))).toBe(true);
    expect(isRefreshShortcut(key('R', { metaKey: true }))).toBe(true);
    expect(isRefreshShortcut(key('r'))).toBe(false);
    expect(isRefreshShortcut(key('r', { ctrlKey: true, shiftKey: true }))).toBe(false);
    expect(isRefreshShortcut(key('a', { ctrlKey: true }))).toBe(false);
  });
});

describe('debounce', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('collapses a burst into one trailing call', () => {
    const fn = vi.fn();
    const d = debounce(fn, 100);
    d.call(); d.call(); d.call();
    vi.advanceTimersByTime(99);
    expect(fn).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it('cancel drops the pending call', () => {
    const fn = vi.fn();
    const d = debounce(fn, 100);
    d.call();
    d.cancel();
    vi.advanceTimersByTime(500);
    expect(fn).not.toHaveBeenCalled();
  });
});

describe('refreshVault', () => {
  const item = { id: 1, type: 'secret', name: 'A' };

  beforeEach(() => {
    invokeMock.mockReset();
    useProjectStore.setState({ projects: [] });
    useVaultStore.setState({ screen: 'projects', items: [], cats: [] });
  });

  it('reloads items, categories and projects', async () => {
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === 'vault_list' ? { items: [item], categories: [{ id: 'c' }] } : [{ id: 7 }]);
    await refreshVault();
    expect(useVaultStore.getState().items).toEqual([item]);
    expect(useVaultStore.getState().cats).toEqual([{ id: 'c' }]);
    expect(useProjectStore.getState().projects).toEqual([{ id: 7 }]);
  });

  it('does nothing on the lock screen', async () => {
    useVaultStore.setState({ screen: 'lock' });
    await refreshVault();
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it('returns to the lock screen when the vault turns out to be locked', async () => {
    invokeMock.mockRejectedValue('vault is locked');
    await refreshVault();
    expect(useVaultStore.getState().screen).toBe('lock');
  });

  it('manualRefresh ignores a second call while one runs', async () => {
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === 'vault_list' ? { items: [], categories: [] } : []);
    await Promise.all([manualRefresh(), manualRefresh()]);
    expect(invokeMock.mock.calls.filter(([c]) => c === 'vault_list')).toHaveLength(1);
  });
});
