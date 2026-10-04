import { useState } from 'react';
import { Icon } from './Icon';
import { useTranslation } from '../../i18n';
import { copySecret } from '../../lib/clipboard';

/**
 * Two-box "code" + "passphrase" display with copy buttons, used after a
 * successful relay upload. Extracted out of `ShareModal.tsx`'s internet-send
 * "done" step so `ProjectShareModal.tsx` (whole-project relay share, issue
 * #4) doesn't duplicate the same markup — behavior is unchanged from the
 * original inline version.
 */
export function RelayCodeDisplay({ code, passphrase }: { code: string; passphrase: string }) {
  const { t } = useTranslation();
  const [copiedCode, setCopiedCode] = useState(false);
  const [copiedPass, setCopiedPass] = useState(false);

  const copyCode = () => {
    copySecret(code).then(() => {
      setCopiedCode(true);
      setTimeout(() => setCopiedCode(false), 2000);
    }).catch(() => {});
  };
  const copyPass = () => {
    copySecret(passphrase).then(() => {
      setCopiedPass(true);
      setTimeout(() => setCopiedPass(false), 2000);
    }).catch(() => {});
  };

  return (
    <>
      <div className="mb-3">
        <div className="text-[9px] font-mono text-tx3 tracking-[0.08em] mb-1">{t('relayCode.code')}</div>
        <div className="bg-raised border border-bd2 rounded-[3px] px-4 py-2.5 flex items-center gap-3">
          <span className="flex-1 font-mono text-[18px] text-accent tracking-[0.3em] font-bold select-all">{code}</span>
          <button
            onClick={copyCode}
            className={[
              'flex items-center gap-1.5 border rounded px-2 py-1 text-[10px] font-mono tracking-wide transition-all cursor-pointer',
              copiedCode ? 'bg-accent-b border-accent-d text-accent' : 'border-bd2 text-tx3 bg-transparent hover:border-tx3',
            ].join(' ')}
          >
            <Icon name={copiedCode ? 'check' : 'copy'} size={10} color={copiedCode ? 'oklch(0.70 0.17 162)' : 'currentColor'} />
            {copiedCode ? t('relayCode.copied') : t('relayCode.copy')}
          </button>
        </div>
      </div>

      <div className="mb-4">
        <div className="text-[9px] font-mono text-tx3 tracking-[0.08em] mb-1">{t('relayCode.passphrase')}</div>
        <div className="bg-raised border border-bd2 rounded-[3px] px-4 py-2.5 flex items-center gap-3">
          <span className="flex-1 font-mono text-[13px] text-accent tracking-[0.05em] select-all break-all">{passphrase}</span>
          <button
            onClick={copyPass}
            className={[
              'flex items-center gap-1.5 border rounded px-2 py-1 text-[10px] font-mono tracking-wide transition-all cursor-pointer',
              copiedPass ? 'bg-accent-b border-accent-d text-accent' : 'border-bd2 text-tx3 bg-transparent hover:border-tx3',
            ].join(' ')}
          >
            <Icon name={copiedPass ? 'check' : 'copy'} size={10} color={copiedPass ? 'oklch(0.70 0.17 162)' : 'currentColor'} />
            {copiedPass ? t('relayCode.copied') : t('relayCode.copy')}
          </button>
        </div>
      </div>

      <p className="text-[10px] text-tx3 font-mono leading-[1.5] mb-3">
        {t('relayCode.expiryNote')}
      </p>
    </>
  );
}
