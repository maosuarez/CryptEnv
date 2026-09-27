import { invoke } from '@tauri-apps/api/core';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import type { WslActionReport, WslError, WslStatus } from '../types';
import { t } from '../i18n';

export const wslKeys = {
  detect: ['wsl', 'detect'] as const,
};

/**
 * On-demand WSL detection (section mount + manual refresh). `wsl_detect` boots
 * each distro to probe it, so it is never refetched in the background.
 */
export function useWslDetect(enabled: boolean) {
  return useQuery({
    queryKey:             wslKeys.detect,
    queryFn:              () => invoke<WslStatus>('wsl_detect'),
    enabled,
    retry:                false,
    staleTime:            Infinity,
    refetchOnWindowFocus: false,
  });
}

function useWslAction(command: 'wsl_configure_client' | 'wsl_remove_client') {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (distro: string) => invoke<WslActionReport>(command, { distro }),
    onSettled:  () => qc.invalidateQueries({ queryKey: wslKeys.detect }),
  });
}

export const useWslConfigure = () => useWslAction('wsl_configure_client');
export const useWslRemove    = () => useWslAction('wsl_remove_client');

/** Human message for a rejected WSL command (typed `{ kind, message }` or anything else). */
export function formatWslError(e: unknown): string {
  if (e && typeof e === 'object' && 'kind' in e) {
    const err = e as WslError;
    switch (err.kind) {
      case 'unsupported':   return t('store.wslUnsupported');
      case 'notAvailable':  return t('store.wslNotAvailable');
      case 'unknownDistro': return t('store.wslUnknownDistro', { distro: err.message ?? '' });
      case 'tooling':       return err.message ? t('store.wslToolingDetail', { message: err.message }) : t('store.wslTooling');
    }
  }
  return e instanceof Error ? e.message : String(e);
}
