import { describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/plugin-os', () => ({ platform: () => 'windows' }));

import { folderTarget, namesEnvFile, resolvedEnvTarget, retargetEnvPath } from './ProjectManager';

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

describe('retargetEnvPath with folder targets', () => {
  it('keeps relative folders: the duplicate gets its own filename there', () => {
    expect(retargetEnvPath('./', 'production', 'staging')).toBe('./');
    expect(retargetEnvPath('apps/web', 'production', 'staging')).toBe('apps/web');
  });
});

describe('folder targets', () => {
  it('tells env files from folders like the backend does', () => {
    for (const f of ['.env', 'apps/api/.env.local', 'prod.env', 'a\\.env.test']) expect(namesEnvFile(f)).toBe(true);
    for (const d of ['./', '.', '', 'apps/web', 'apps\\web\\']) expect(namesEnvFile(d)).toBe(false);
  });

  it('only relative non-env-file paths are folders', () => {
    expect(folderTarget('./')).toBe('./');
    expect(folderTarget('apps/web')).toBe('apps/web');
    expect(folderTarget('.env')).toBeNull();
    expect(folderTarget('/abs/dir')).toBeNull();
    expect(folderTarget('C:\\proj\\dir')).toBeNull();
  });

  it('previews the file a folder injects into', () => {
    expect(resolvedEnvTarget('./', 'production')).toBe('.env.production');
    expect(resolvedEnvTarget('apps/web/', 'staging')).toBe('apps/web/.env.staging');
    expect(resolvedEnvTarget('apps/web', 'default')).toBe('apps/web/.env');
    expect(resolvedEnvTarget('apps/web', '')).toBe('apps/web/.env');
    expect(resolvedEnvTarget('apps/api/.env.custom', 'production')).toBe('apps/api/.env.custom');
    expect(resolvedEnvTarget('/abs/dir', 'production')).toBe('/abs/dir');
  });
});
