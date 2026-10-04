import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from '../i18n';

const BTN = 'h-9 px-4 rounded-[3px] text-[12px] font-semibold tracking-[0.06em] font-ui cursor-pointer transition-colors disabled:opacity-40 disabled:cursor-default';

/** Shown instead of the app when the vault database could not be opened at
 *  startup. "Move aside" renames the file (never deletes it) and restarts. */
export function RecoveryScreen({ error }: { error: string }) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState('');

  const moveAside = async () => {
    setBusy(true);
    setFailure('');
    try {
      await invoke('recovery_move_aside');
    } catch (e: unknown) {
      setFailure(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  };

  return (
    <div className="flex flex-col w-full h-full bg-bg items-center justify-center px-10">
      <div className="w-full max-w-[520px] flex flex-col gap-4">
        <div className="text-[14px] font-mono text-danger tracking-[0.06em]">{t('recovery.title')}</div>
        <div className="text-[13px] text-tx2 leading-relaxed">{t('recovery.body')}</div>
        <div className="text-[12px] font-mono text-tx3 break-words">{t('recovery.details', { error })}</div>
        {failure && (
          <div role="alert" className="text-[12px] font-mono text-danger break-words">
            {t('recovery.failed', { error: failure })}
          </div>
        )}
        <div className="flex gap-3 pt-2">
          <button
            onClick={moveAside}
            disabled={busy}
            className={`${BTN} bg-accent-b border border-accent-d text-accent hover:text-tx`}
          >
            {t('recovery.moveAside')}
          </button>
          <button
            onClick={() => { invoke('recovery_quit').catch(() => {}); }}
            disabled={busy}
            className={`${BTN} bg-transparent border border-bd2 text-tx2 hover:text-tx`}
          >
            {t('recovery.quit')}
          </button>
        </div>
      </div>
    </div>
  );
}
