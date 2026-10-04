import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useVaultStore } from '../../store';
import { useTranslation } from '../../i18n';

/** Non-fatal notice shown after unlock when the global hotkey could not be
 *  registered at startup (another app owns the combination). */
export function HotkeyNotice() {
  const screen = useVaultStore((s) => s.screen);
  const { t } = useTranslation();
  const [unavailable, setUnavailable] = useState(false);
  const [dismissed, setDismissed] = useState(false);

  useEffect(() => {
    invoke<{ unavailable: boolean }>('hotkey_status')
      .then((s) => setUnavailable(s.unavailable))
      .catch(() => {});
  }, [screen]);

  if (!unavailable || dismissed || screen === 'lock') return null;

  return (
    <div
      role="status"
      className="fixed bottom-14 left-4 z-[8000] w-[300px] bg-raised border border-bd2 rounded-[4px] px-[14px] py-[10px] shadow-[0_4px_20px_rgba(0,0,0,.6)] animate-fade-in"
    >
      <div className="text-[12px] font-mono text-tx2">{t('hotkeyNotice.unavailable')}</div>
      <div className="mt-2 flex justify-end">
        <button
          onClick={() => setDismissed(true)}
          className="h-7 px-3 rounded-[3px] text-[11px] font-semibold tracking-[0.06em] font-ui cursor-pointer bg-transparent border border-bd2 text-tx2 hover:text-tx transition-colors"
        >
          {t('hotkeyNotice.dismiss')}
        </button>
      </div>
    </div>
  );
}
