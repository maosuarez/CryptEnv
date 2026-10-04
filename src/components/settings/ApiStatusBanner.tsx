import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useVaultStore } from '../../store';
import { useTranslation } from '../../i18n';

type ApiStatus =
  | { state: 'starting' | 'running' | 'stopped' }
  | { state: 'failed'; code: string; reason: string };

const KNOWN_FAILURES = ['port_in_use', 'tls_error', 'bind_error', 'server_error'] as const;

const BTN =
  'h-7 px-3 bg-transparent border border-bd2 rounded-[3px] text-tx2 text-[11px] cursor-pointer font-ui font-semibold tracking-[0.06em] hover:text-tx transition-colors disabled:opacity-40';

/**
 * Settings banner shown when the local REST server is not running, with the
 * reason and a "Regenerate certificate" action (restarts the server). Renders
 * nothing while the API is healthy.
 */
export function ApiStatusBanner() {
  const { t } = useTranslation();
  const showToast = useVaultStore((s) => s.showToast);
  const [status, setStatus] = useState<ApiStatus | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setStatus(await invoke<ApiStatus>('api_status'));
    } catch {
      setStatus(null);
    }
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);

  if (!status || status.state === 'running' || status.state === 'starting') return null;

  const reasonKey =
    status.state === 'failed' && (KNOWN_FAILURES as readonly string[]).includes(status.code)
      ? (status.code as (typeof KNOWN_FAILURES)[number])
      : status.state === 'failed' ? 'server_error' : 'stopped';

  const regenerate = async () => {
    setBusy(true);
    try {
      const next = await invoke<ApiStatus>('tls_regenerate');
      setStatus(next);
      if (next.state === 'running') showToast(t('apiStatus.regenerated'));
    } catch (e) {
      showToast(t('apiStatus.regenerateFailed', { error: String(e) }), 'error');
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      role="alert"
      data-testid="api-status-banner"
      className="mt-4 mb-2 px-3 py-2 rounded-[3px] border border-danger bg-raised"
    >
      <div className="text-[11px] font-semibold font-mono tracking-[0.08em] text-danger">
        {t('apiStatus.title')}
      </div>
      <div className="mt-1 text-[12px] font-mono text-tx2">{t(`apiStatus.reason.${reasonKey}`)}</div>
      <div className="mt-1 text-[11px] font-mono text-tx3">{t('apiStatus.impact')}</div>
      <button onClick={regenerate} disabled={busy} className={`${BTN} mt-2`}>
        {busy ? t('apiStatus.regenerating') : t('apiStatus.regenerate')}
      </button>
    </div>
  );
}
