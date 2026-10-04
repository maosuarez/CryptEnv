import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue({ autoLockTimeout: 5, hotkey: 'Ctrl+Alt+Z' }) }));

import { useVaultStore } from './index';
import type { VaultItem } from '../types';

const cmd = { id: 1, type: 'command', name: 'deploy', command: 'run {{token}}' } as unknown as VaultItem;

function openSecretUi() {
  useVaultStore.setState({
    screen: 'projects',
    items: [cmd],
    placeholder: cmd,
    wizardOpen: true,
    editTarget: cmd,
    menu: { x: 0, y: 0, items: [] } as never,
  });
}

function expectClean() {
  const s = useVaultStore.getState();
  expect(s.screen).toBe('lock');
  expect(s.placeholder).toBeNull();
  expect(s.wizardOpen).toBe(false);
  expect(s.editTarget).toBeNull();
  expect(s.menu).toBeNull();
  expect(s.items).toEqual([]);
}

beforeEach(openSecretUi);

describe('lock clears secret-bearing UI state', () => {
  it('user lock', async () => {
    await useVaultStore.getState().lock();
    expectClean();
  });

  it('backend-initiated lock (auto-lock)', () => {
    useVaultStore.getState().lockedByBackend();
    expectClean();
  });

  it('wipe', async () => {
    await useVaultStore.getState().wipe();
    expectClean();
  });

  it('modal is not restored after the next unlock', async () => {
    useVaultStore.getState().lockedByBackend();
    await useVaultStore.getState().unlockWithPayload({ items: [], categories: [] });
    expect(useVaultStore.getState().placeholder).toBeNull();
    expect(useVaultStore.getState().wizardOpen).toBe(false);
  });
});
