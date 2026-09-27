import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getVersion } from '@tauri-apps/api/app';

/** Mirrors `AppSystemInfo` in src-tauri/src/vault/mod.rs. Non-sensitive. */
export interface SystemInfo {
  version: string;
  appDir:  string;
  dbPath:  string;
  os:      string;
}

let cached: Promise<Partial<SystemInfo>> | null = null;

/** Resolves runtime diagnostics once per launch. Falls back to the bundle
 *  version from `@tauri-apps/api/app` if the backend command fails. */
function load(): Promise<Partial<SystemInfo>> {
  cached ??= invoke<SystemInfo>('app_get_system_info').catch(async () => {
    try {
      return { version: await getVersion() };
    } catch {
      return {};
    }
  });
  return cached;
}

export function useSystemInfo(): Partial<SystemInfo> {
  const [info, setInfo] = useState<Partial<SystemInfo>>({});
  useEffect(() => {
    let alive = true;
    load().then((i) => { if (alive) setInfo(i); });
    return () => { alive = false; };
  }, []);
  return info;
}
