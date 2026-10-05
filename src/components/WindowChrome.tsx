import { useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { platform } from '@tauri-apps/plugin-os';
import { useQueryClient } from '@tanstack/react-query';
import { Icon } from './ui/Icon';
import { useVaultStore } from '../store';
import { manualRefresh, useRefreshing } from '../lib/vaultRefresh';
import { useTranslation } from '../i18n';

const win = getCurrentWindow();

type Chrome = { showControls: boolean };

function chromeFor(os: string): Chrome {
  switch (os) {
    case 'windows':
    case 'linux':
      return { showControls: true };
    default:            // macos and anything else: unchanged behaviour
      return { showControls: false };
  }
}

export function WindowChrome() {
  const screen = useVaultStore((s) => s.screen);
  const lock   = useVaultStore((s) => s.lock);
  const [chrome] = useState(() => chromeFor(platform()));
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const refreshing  = useRefreshing((s) => s.refreshing);

  return (
    <div
      className="relative flex items-center h-12 bg-bg border-b border-bd select-none shrink-0"
    >
      <div className="absolute inset-0 flex items-center justify-center pointer-events-none">
        <div className="flex items-center gap-2 text-xs font-medium tracking-widest text-tx3 font-mono">
          <Icon name="shield" size={14} color="oklch(0.70 0.17 162)" />
          CRYPTENV
        </div>
      </div>

      <div data-tauri-drag-region className="flex-1 h-full" />

      {screen !== 'lock' && (
        <button
          onClick={() => { void manualRefresh(queryClient); }}
          disabled={refreshing}
          title={t('window.refresh')}
          aria-label={t('window.refresh')}
          className={[
            'relative z-10 flex items-center justify-center w-7 h-7 mr-1 rounded-[3px]',
            'bg-transparent border-none cursor-pointer text-tx3',
            'hover:text-tx transition-colors duration-150 disabled:opacity-50',
          ].join(' ')}
        >
          <span className={refreshing ? 'animate-spin inline-flex' : 'inline-flex'}>
            <Icon name="refresh" size={14} />
          </span>
        </button>
      )}

      {screen !== 'lock' && (
        <button
          onClick={() => lock()}
          className={[
            'relative z-10 flex items-center gap-1.5 px-2.5 py-1 mr-2 rounded-[3px]',
            'bg-transparent border-none cursor-pointer',
            'text-xs font-ui tracking-wider text-tx3',
            'hover:text-danger transition-colors duration-150',
          ].join(' ')}
        >
          <Icon name="lock" size={13} />
          {t('window.lock')}
        </button>
      )}

      {chrome.showControls && (
        <div className="relative z-10 flex h-full items-stretch border-l border-bd">
          <button
            onClick={() => win.minimize()}
            title={t('window.minimize')}
            aria-label={t('window.minimize')}
            className={[
              'flex items-center justify-center w-12 h-full border-none cursor-pointer',
              'bg-transparent text-tx3 hover:text-tx hover:bg-surface transition-colors duration-100',
              'focus-visible:outline focus-visible:outline-1 focus-visible:outline-bd focus-visible:-outline-offset-1',
            ].join(' ')}
          >
            <svg width="10" height="1" viewBox="0 0 10 1" fill="currentColor">
              <rect width="10" height="1" />
            </svg>
          </button>
          <button
            onClick={() => win.close()}
            title={t('common.close')}
            aria-label={t('common.close')}
            className={[
              'flex items-center justify-center w-12 h-full border-none cursor-pointer',
              'bg-transparent text-tx3 hover:text-tx hover:bg-danger transition-colors duration-100',
              'focus-visible:outline focus-visible:outline-1 focus-visible:outline-bd focus-visible:-outline-offset-1',
            ].join(' ')}
          >
            <svg width="10" height="10" viewBox="0 0 10 10" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round">
              <line x1="0" y1="0" x2="10" y2="10" />
              <line x1="10" y1="0" x2="0" y2="10" />
            </svg>
          </button>
        </div>
      )}
    </div>
  );
}
