import { beforeEach, describe, expect, it, vi } from 'vitest';

const invokeMock = vi.fn();
const writeTextMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock('@tauri-apps/plugin-clipboard-manager', () => ({ writeText: (...a: unknown[]) => writeTextMock(...a) }));

import { copyPlain, copySecret } from './clipboard';
import { useVaultStore } from '../store';

beforeEach(() => {
  invokeMock.mockReset();
  writeTextMock.mockReset();
  useVaultStore.setState({ toast: null });
});

describe('copySecret / copyPlain', () => {
  it('copySecret goes through the backend command and never the plugin', async () => {
    invokeMock.mockResolvedValue(undefined);
    await copySecret('hunter2');
    expect(invokeMock).toHaveBeenCalledWith('clipboard_write_secret', { text: 'hunter2' });
    expect(writeTextMock).not.toHaveBeenCalled();
    expect(useVaultStore.getState().toast?.msg).toContain('30 s');
  });

  it('copySecret shows no toast when the backend write fails', async () => {
    invokeMock.mockRejectedValue(new Error('boom'));
    await expect(copySecret('x')).rejects.toThrow();
    expect(useVaultStore.getState().toast).toBeNull();
  });

  it('copyPlain uses the plugin and not the secret command', async () => {
    writeTextMock.mockResolvedValue(undefined);
    await copyPlain('project-name');
    expect(writeTextMock).toHaveBeenCalledWith('project-name');
    expect(invokeMock).not.toHaveBeenCalled();
  });
});

/** Guard: raw clipboard writes bypass the secret lifetime, so only
 *  lib/clipboard.ts may touch the plugin or `navigator.clipboard`. */
describe('clipboard write guard', () => {
  const files = import.meta.glob<string>(['/src/**/*.{ts,tsx}', '!/src/**/*.test.{ts,tsx}'], {
    query: '?raw',
    import: 'default',
    eager: true,
  });
  const allowed = '/src/lib/clipboard.ts';
  const forbidden = [/plugin-clipboard-manager/, /navigator\.clipboard\.writeText/];

  it('no source file outside lib/clipboard.ts writes to the clipboard directly', () => {
    const offenders = Object.entries(files)
      .filter(([path, text]) => path !== allowed && forbidden.some((re) => re.test(text)))
      .map(([path]) => path);
    expect(Object.keys(files)).toContain(allowed);
    expect(offenders).toEqual([]);
  });
});
