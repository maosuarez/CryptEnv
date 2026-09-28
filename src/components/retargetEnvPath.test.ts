import { describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/plugin-os', () => ({ platform: () => 'windows' }));

import { retargetEnvPath } from './ProjectManager';

describe('retargetEnvPath (environment duplication)', () => {
  it('swaps the filename when it is the source environment file', () => {
    expect(retargetEnvPath('C:\\app\\.env.local', 'local', 'production')).toBe('C:\\app\\.env.production');
    expect(retargetEnvPath('/home/me/app/.env', '', 'staging')).toBe('/home/me/app/.env.staging');
    expect(retargetEnvPath('/app/.env', 'default', 'test')).toBe('/app/.env.test');
    expect(retargetEnvPath('/app/.env.staging', 'staging', '')).toBe('/app/.env');
  });

  it('drops paths not tied to the source environment file', () => {
    expect(retargetEnvPath('/app/config.env', 'local', 'production')).toBeNull();
    expect(retargetEnvPath('/app/.env.production', 'local', 'staging')).toBeNull();
  });
});
