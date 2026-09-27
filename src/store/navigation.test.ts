import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

import { HISTORY_LIMIT, useVaultStore } from './index';

const nav = () => useVaultStore.getState();

beforeEach(() => {
  useVaultStore.setState({ screen: 'projects', history: [] });
});

describe('vault store navigation history', () => {
  it('returns to Projects when Settings was opened from Projects', () => {
    nav().go('settings');
    nav().goBack();
    expect(nav().screen).toBe('projects');
  });

  it('returns to Global Secrets when Settings was opened from Global Secrets', () => {
    nav().go('vault');
    nav().go('settings');
    nav().goBack();
    expect(nav().screen).toBe('vault');
  });

  it('unwinds nested screens in order', () => {
    nav().go('vault');
    nav().go('settings');
    nav().go('categories');
    nav().goBack();
    expect(nav().screen).toBe('settings');
    nav().goBack();
    expect(nav().screen).toBe('vault');
    nav().goBack();
    expect(nav().screen).toBe('projects');
  });

  it('falls back to Projects when history is empty', () => {
    useVaultStore.setState({ screen: 'settings', history: [] });
    nav().goBack();
    expect(nav().screen).toBe('projects');
  });

  it('never returns to the lock screen', () => {
    useVaultStore.setState({ screen: 'settings', history: ['lock'] });
    nav().goBack();
    expect(nav().screen).toBe('projects');
  });

  it('ignores navigation to the current screen', () => {
    nav().go('projects');
    expect(nav().history).toEqual([]);
  });

  it('caps history length', () => {
    for (let i = 0; i < HISTORY_LIMIT * 2; i++) nav().go(i % 2 ? 'projects' : 'vault');
    expect(nav().history.length).toBeLessThanOrEqual(HISTORY_LIMIT);
  });
});
