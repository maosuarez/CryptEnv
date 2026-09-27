import { useState } from 'react';
import { writeText } from '@tauri-apps/plugin-clipboard-manager';
import { Icon } from './Icon';
import { useTranslation } from '../../i18n';

interface CopyBtnProps {
  value:  string;
  label?: string;
  title?: string;
}

export function CopyBtn({ value, label, title }: CopyBtnProps) {
  const [ok, setOk] = useState(false);
  const { t } = useTranslation();

  const handle = async (e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      await writeText(value);
      setOk(true);
      setTimeout(() => setOk(false), 1800);
    } catch {}
  };

  return (
    <button
      onClick={handle}
      title={title}
      className={[
        'flex items-center gap-1 rounded-[3px] px-2 py-1',
        'border text-xs font-medium tracking-wide font-ui',
        'transition-all duration-150 shrink-0 whitespace-nowrap cursor-pointer',
        ok
          ? 'bg-accent-b border-accent-d text-accent'
          : 'bg-transparent border-bd text-tx3 hover:border-bd2 hover:text-tx',
      ].join(' ')}
    >
      <Icon name={ok ? 'check' : 'copy'} size={12} />
      {ok ? t('ui.ok') : (label ?? t('rows.copy'))}
    </button>
  );
}
