import { useState } from 'react';
import { Icon } from './Icon';
import { CmdHL } from './CmdHL';
import { useTranslation } from '../../i18n';

interface PlaceholderModalProps {
  command: string;
  onClose: () => void;
}

export function PlaceholderModal({ command, onClose }: PlaceholderModalProps) {
  const phs = [...new Set([...command.matchAll(/\{\{(\w+)\}\}/g)].map((m) => m[1]))];
  const [vals, setVals] = useState<Record<string, string>>(
    Object.fromEntries(phs.map((p) => [p, '']))
  );
  const [ok, setOk] = useState(false);
  const { t } = useTranslation();

  const filled = command.replace(/\{\{(\w+)\}\}/g, (_, k) => vals[k] || `{{${k}}}`);

  const handleCopy = () => {
    // TODO: wire to clipboard-manager writeText(filled)
    setOk(true);
    setTimeout(() => setOk(false), 1800);
  };

  return (
    <div
      onClick={onClose}
      className="fixed inset-0 bg-[rgba(4,5,6,.85)] flex items-center justify-center z-[9000] backdrop-blur-[4px]"
    >
      <div
        onClick={(e) => e.stopPropagation()}
        className={[
          'bg-surface border border-bd2 rounded-[4px] p-5 w-[380px]',
          'shadow-[0_16px_48px_rgba(0,0,0,.8)] animate-fade-in',
        ].join(' ')}
      >
        <div className="flex items-center justify-between mb-4">
          <div className="text-[13px] font-semibold text-tx flex items-center gap-[7px]">
            <Icon name="terminal" size={13} color="oklch(0.72 0.16 68)" />
            {t('ui.fillPlaceholders')}
          </div>
          <button
            onClick={onClose}
            aria-label={t('common.close')}
            className="bg-transparent border-none cursor-pointer text-tx3 flex p-[2px] hover:text-tx transition-colors"
          >
            <Icon name="close" size={13} />
          </button>
        </div>

        {phs.map((p) => (
          <div key={p} className="mb-[10px]">
            <div className="text-[10px] text-warn font-mono mb-1 tracking-[0.05em]">{`{{${p}}}`}</div>
            <input
              value={vals[p]}
              onChange={(e) => setVals((v) => ({ ...v, [p]: e.target.value }))}
              placeholder={t('ui.valueFor', { name: p })}
              className={[
                'w-full px-[10px] py-[7px] bg-raised border border-bd2',
                'rounded-[3px] text-[12px] font-mono text-tx',
                'focus:border-accent-d outline-none transition-colors',
              ].join(' ')}
            />
          </div>
        ))}

        <div className="mt-[14px] px-[10px] py-[9px] bg-raised border border-bd rounded-[3px] text-[11px] font-mono text-tx2 break-all leading-[1.7]">
          <CmdHL cmd={filled} />
        </div>

        <button
          onClick={handleCopy}
          className={[
            'mt-3 w-full py-[9px] rounded-[3px] text-[12px] font-bold tracking-[0.06em]',
            'cursor-pointer font-ui flex items-center justify-center gap-1.5 transition-all duration-150',
            ok
              ? 'bg-accent-b border border-accent-d text-accent'
              : 'bg-accent border-none text-[#020504]',
          ].join(' ')}
        >
          <Icon
            name={ok ? 'check' : 'copy'}
            size={12}
            color={ok ? 'oklch(0.70 0.17 162)' : '#020504'}
          />
          {ok ? t('ui.copiedBang') : t('ui.copyFilled')}
        </button>
      </div>
    </div>
  );
}
