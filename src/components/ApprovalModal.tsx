import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Icon } from './ui/Icon';
import { RelayCodeDisplay } from './ui/RelayCodeDisplay';
import { useVaultStore } from '../store';
import { useTranslation } from '../i18n';
import type { ApprovalResolution, PendingApproval } from '../types';

const BTN = 'h-8 px-4 rounded-[3px] text-[11px] font-semibold tracking-[0.06em] font-ui cursor-pointer transition-colors disabled:opacity-40 disabled:cursor-default';

/**
 * Human approval for sensitive MCP requests (relay send, share export, MCP host
 * config writes, generate_env). The backend only acts after the user clicks
 * APPROVE here; any relay code / passphrase is shown in this window only and is
 * never returned to the MCP caller. Deny is focused by default.
 */
export function ApprovalModal() {
  const screen = useVaultStore((s) => s.screen);
  const { t } = useTranslation();
  const [pending, setPending] = useState<PendingApproval[]>([]);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<ApprovalResolution | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const loadedAt = useRef(Date.now());
  const denyRef = useRef<HTMLButtonElement>(null);

  const refresh = useCallback(() => {
    invoke<PendingApproval[]>('approval_list')
      .then((list) => { loadedAt.current = Date.now(); setPending(list); })
      .catch(() => setPending([]));
  }, []);

  // New request pushed by the backend.
  useEffect(() => {
    const unlisten = listen('approval://requested', refresh);
    return () => { unlisten.then((f) => f()).catch(() => {}); };
  }, [refresh]);

  // Catch requests raised while the window was closed or the vault locked, and
  // drop expired ones; a request lives at most 120 s so a slow poll is enough.
  useEffect(() => {
    if (screen === 'lock') { setPending([]); setResult(null); setMessage(null); return; }
    refresh();
    const id = setInterval(() => { refresh(); setNow(Date.now()); }, 3000);
    const tick = setInterval(() => setNow(Date.now()), 1000);
    return () => { clearInterval(id); clearInterval(tick); };
  }, [screen, refresh]);

  const current = pending[0];

  useEffect(() => {
    if (current && !result && !message) denyRef.current?.focus();
  }, [current?.id, result, message]);

  const resolve = async (approve: boolean) => {
    if (!current) return;
    setBusy(true);
    try {
      const res = await invoke<ApprovalResolution>('approval_resolve', { id: current.id, approve });
      setPending((list) => list.filter((a) => a.id !== current.id));
      if (!approve) {
        setMessage(t('approval.denied'));
      } else if (res.meta && res.meta.ok === false) {
        setMessage(t('approval.failed', { error: String(res.meta.error ?? '') }));
      } else {
        setResult(res);
      }
    } catch (e) {
      setMessage(String(e));
      refresh();
    } finally {
      setBusy(false);
    }
  };

  const dismiss = () => { setResult(null); setMessage(null); };

  if (screen === 'lock') return null;
  if (!current && !result && !message) return null;

  const secondsLeft = current
    ? Math.max(0, current.expiresIn - Math.floor((now - loadedAt.current) / 1000))
    : 0;

  return (
    <div
      role="dialog"
      aria-modal="true"
      className="fixed inset-0 bg-[rgba(4,5,6,.88)] flex items-center justify-center z-[9500] backdrop-blur-[4px]"
    >
      <div className="bg-surface border border-bd2 rounded-[4px] p-5 w-[440px] max-h-[88vh] overflow-y-auto shadow-[0_16px_48px_rgba(0,0,0,.8)] animate-fade-in">
        {result ? (
          <>
            <div className="text-[13px] font-semibold text-tx flex items-center gap-[7px] mb-3">
              <Icon name="check" size={14} color="oklch(0.70 0.17 162)" />
              {t('approval.doneTitle')}
            </div>
            <div className="text-[12px] text-tx2 font-mono mb-3 break-words">{result.summary.operation}</div>
            {result.secret?.type === 'relay' && (
              <>
                <RelayCodeDisplay code={result.secret.code} passphrase={result.secret.passphrase} />
                <p className="text-[11px] text-tx3 font-mono mb-3">{t('approval.relayDone')}</p>
              </>
            )}
            {result.secret?.type === 'export' && (
              <>
                <div className="text-[9px] font-mono text-tx3 tracking-[0.08em] mb-1">{t('approval.passphrase')}</div>
                <div className="bg-raised border border-bd2 rounded-[3px] px-4 py-2.5 mb-3 font-mono text-[13px] text-accent break-all select-all">
                  {result.secret.passphrase}
                </div>
                <p className="text-[11px] text-tx3 font-mono mb-3 break-all">
                  {t('approval.exportDone', { path: result.secret.path })}
                </p>
              </>
            )}
            {!result.secret && (
              <p className="text-[11px] text-tx3 font-mono mb-3 break-all">
                {typeof result.meta.path === 'string' ? result.meta.path : ''}
              </p>
            )}
            <div className="flex justify-end">
              <button onClick={dismiss} className={`${BTN} bg-accent-b border border-accent-d text-accent hover:text-tx`}>
                {t('approval.close')}
              </button>
            </div>
          </>
        ) : message ? (
          <>
            <p className="text-[12px] text-tx2 font-mono mb-4 break-words">{message}</p>
            <div className="flex justify-end">
              <button onClick={dismiss} className={`${BTN} bg-transparent border border-bd2 text-tx2 hover:text-tx`}>
                {t('approval.close')}
              </button>
            </div>
          </>
        ) : current && (
          <>
            <div className="flex items-center justify-between mb-3">
              <div className="text-[13px] font-semibold text-tx tracking-[0.04em]">{t('approval.title')}</div>
              <div className="text-[10px] font-mono text-tx3">
                {t('approval.expiresIn', { s: secondsLeft })}
              </div>
            </div>

            <div className="text-[10px] font-mono text-tx3 tracking-[0.06em] mb-1">{t('approval.source')}</div>
            <div className="text-[13px] text-accent font-mono mb-3 break-words">{current.summary.operation}</div>

            {current.summary.items.length > 0 && (
              <div className="mb-3">
                <div className="text-[9px] font-mono text-tx3 tracking-[0.08em] mb-1">
                  {t('approval.items')} ({current.summary.item_count})
                </div>
                <div className="bg-raised border border-bd2 rounded-[3px] px-3 py-2 font-mono text-[12px] text-tx max-h-[120px] overflow-y-auto break-all">
                  {current.summary.items.join(', ')}
                </div>
              </div>
            )}

            {current.summary.destination && (
              <div className="mb-3">
                <div className="text-[9px] font-mono text-tx3 tracking-[0.08em] mb-1">{t('approval.destination')}</div>
                <div className="bg-raised border border-bd2 rounded-[3px] px-3 py-2 font-mono text-[12px] text-tx break-all">
                  {current.summary.destination}
                </div>
              </div>
            )}

            {current.summary.details.length > 0 && (
              <div className="mb-3">
                <div className="text-[9px] font-mono text-tx3 tracking-[0.08em] mb-1">{t('approval.details')}</div>
                <div className="bg-raised border border-bd2 rounded-[3px] px-3 py-2 font-mono text-[12px] text-tx2 break-all">
                  {current.summary.details.map((d, i) => <div key={i}>{d}</div>)}
                </div>
              </div>
            )}

            <p className="text-[11px] text-tx3 font-mono mb-4">{t('approval.warning')}</p>
            {pending.length > 1 && (
              <p className="text-[10px] text-tx3 font-mono mb-3">{t('approval.pendingMore', { n: pending.length - 1 })}</p>
            )}

            <div className="flex justify-end gap-2">
              <button
                ref={denyRef}
                onClick={() => resolve(false)}
                disabled={busy}
                className={`${BTN} bg-transparent border border-bd2 text-tx hover:border-tx3`}
              >
                {t('approval.deny')}
              </button>
              <button
                onClick={() => resolve(true)}
                disabled={busy || secondsLeft === 0}
                className={`${BTN} bg-accent-b border border-accent-d text-accent hover:text-tx`}
              >
                {busy ? t('approval.working') : t('approval.approve')}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
