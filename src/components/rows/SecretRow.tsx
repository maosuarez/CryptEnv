import { useState } from 'react';
import { writeText } from '@tauri-apps/plugin-clipboard-manager';
import { Icon } from '../ui/Icon';
import { CopyBtn } from '../ui/CopyBtn';
import { KebabBtn } from '../ui/KebabBtn';
import { ContextMenu } from '../ui/ContextMenu';
import { useVaultStore } from '../../store';
import { useTranslation } from '../../i18n';
import type { SecretItem, Category } from '../../types';

interface Props {
  item:      SecretItem;
  cats:      Category[];
  selected?: boolean;
  onToggle?: (id: number) => void;
  onShare?:  (id: number) => void;
  onSelect?: (id: number) => void;
}

const BADGE_VISIBLE_MAX = 2;

export function CatBadges({ names, cats }: { names: string[]; cats: Category[] }) {
  const [expanded, setExpanded] = useState(false);
  const { t } = useTranslation();

  if (names.length === 0) return null;

  const visible  = expanded ? names : names.slice(0, BADGE_VISIBLE_MAX);
  const overflow = names.length - BADGE_VISIBLE_MAX;

  return (
    <div className="flex items-center gap-1 flex-wrap shrink-0 max-w-[220px]">
      {visible.map((name) => {
        const c = cats.find((x) => x.name === name);
        const color = c?.color ?? 'var(--color-tx4)';
        return (
          <span
            key={name}
            title={name}
            className="inline-flex items-center px-1.5 py-0.5 rounded-[2px] border text-[0.6rem] font-mono tracking-wide font-medium shrink-0 leading-none whitespace-nowrap"
            style={{
              borderColor: color,
              color:        color,
              background:   `color-mix(in oklch, ${color} 12%, transparent)`,
            }}
          >
            {name}
          </span>
        );
      })}
      {!expanded && overflow > 0 && (
        <button
          onClick={(e) => { e.stopPropagation(); setExpanded(true); }}
          className="inline-flex items-center px-1.5 py-0.5 rounded-[2px] border border-bd2 bg-raised text-tx3 text-[0.6rem] font-mono tracking-wide shrink-0 leading-none cursor-pointer hover:border-bd hover:text-tx2 transition-colors"
        >
          +{overflow}
        </button>
      )}
      {expanded && names.length > BADGE_VISIBLE_MAX && (
        <button
          onClick={(e) => { e.stopPropagation(); setExpanded(false); }}
          className="inline-flex items-center px-1.5 py-0.5 rounded-[2px] border border-bd2 bg-raised text-tx3 text-[0.6rem] font-mono tracking-wide shrink-0 leading-none cursor-pointer hover:border-bd hover:text-tx2 transition-colors"
        >
          {t('rows.less')}
        </button>
      )}
    </div>
  );
}

/** @deprecated Use CatBadges instead */
function CatDots({ names, cats }: { names: string[]; cats: Category[] }) {
  return <CatBadges names={names} cats={cats} />;
}

export function SecretRow({ item, cats, selected, onToggle, onShare, onSelect }: Props) {
  const [rev, setRev] = useState(false);
  const [ctx, setCtx] = useState<{ x: number; y: number } | null>(null);
  const go           = useVaultStore((s) => s.go);
  const setEditTarget = useVaultStore((s) => s.setEditTarget);
  const deleteItem   = useVaultStore((s) => s.deleteItem);
  const showToast    = useVaultStore((s) => s.showToast);
  const { t }        = useTranslation();

  const copyAs = async (fmt: 'env' | 'bash' | 'ps1') => {
    const text =
      fmt === 'env'  ? `${item.name}=${item.value}` :
      fmt === 'bash' ? `export ${item.name}=${item.value}` :
                      `$env:${item.name} = "${item.value}"`;
    try {
      await writeText(text);
      showToast(t('rows.copiedAs', { fmt: fmt === 'ps1' ? 'PowerShell' : fmt }));
    } catch {
      showToast(t('rows.clipboardError'));
    }
  };

  const kebab = [
    { icon: 'edit',   label: t('common.edit'),   onClick: () => { setEditTarget(item); go('edit'); } },
    { divider: true },
    { icon: 'export', label: t('rows.copyAs', { fmt: '.env' }),        sub: '.env', onClick: () => copyAs('env')  },
    { icon: 'export', label: t('rows.copyAs', { fmt: 'bash' }),        sub: 'bash', onClick: () => copyAs('bash') },
    { icon: 'export', label: t('rows.copyAs', { fmt: 'PowerShell' }),  sub: 'ps1',  onClick: () => copyAs('ps1')  },
    { divider: true },
    { icon: 'trash',  label: t('common.deleteLower'), danger: true, onClick: () => deleteItem(item.id) },
  ];

  const ctxItems = [
    { icon: 'export', label: t('rows.shareItem'),   onClick: () => onShare?.(item.id) },
    { icon: 'check',  label: t('rows.selectForSharing'), onClick: () => onSelect?.(item.id) },
  ];

  return (
    <div
      className={[
        'px-5 py-4 border-b border-bd transition-colors duration-100',
        selected ? 'bg-accent-b' : 'hover:bg-raised',
        onToggle ? 'cursor-pointer' : '',
      ].join(' ')}
      onClick={onToggle ? () => onToggle(item.id) : undefined}
      onContextMenu={(e) => { e.preventDefault(); setCtx({ x: e.clientX, y: e.clientY }); }}
    >
      {/* Name row */}
      <div className="flex items-center gap-3 mb-2">
        {onToggle ? (
          <span
            className={[
              'w-4 h-4 rounded-[3px] border shrink-0 flex items-center justify-center transition-all duration-100',
              selected ? 'bg-accent border-accent-d' : 'border-bd2 bg-transparent',
            ].join(' ')}
            onClick={(e) => { e.stopPropagation(); onToggle(item.id); }}
          >
            {selected && (
              <svg width="10" height="10" viewBox="0 0 16 16" fill="none" stroke="#020504" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                <path d="M2.5 8.5l4 4 7-8" />
              </svg>
            )}
          </span>
        ) : (
          <span className="w-2 h-2 rounded-full bg-accent shrink-0" />
        )}
        <span className="flex-1 text-[13px] font-semibold font-mono text-tx overflow-hidden text-ellipsis whitespace-nowrap">
          {item.name}
        </span>
        <CatBadges names={item.categories} cats={cats} />
      </div>
      {/* Value row */}
      <div className="flex items-center gap-2 pl-5">
        <span
          className={[
            'flex-1 text-xs font-mono overflow-hidden text-ellipsis whitespace-nowrap',
            rev ? 'text-tx tracking-[0.02em]' : 'text-tx3 tracking-[0.1em]',
          ].join(' ')}
        >
          {rev ? item.value : '••••••••••••••••••••'}
        </span>
        <button
          onClick={(e) => { e.stopPropagation(); setRev((v) => !v); }}
          aria-label={rev ? t('common.hide') : t('common.show')}
          className="border border-bd bg-transparent text-tx3 cursor-pointer p-1 rounded-[3px] flex shrink-0 hover:border-bd2 hover:text-tx transition-all duration-150"
        >
          <Icon name={rev ? 'eyeOff' : 'eye'} size={13} />
        </button>
        <CopyBtn value={item.value} />
        <KebabBtn menuItems={kebab} />
      </div>
      {item.notes && (
        <div className="mt-2 pl-5 text-xs text-tx3 overflow-hidden text-ellipsis whitespace-nowrap italic">
          {item.notes}
        </div>
      )}
      {ctx && (
        <ContextMenu x={ctx.x} y={ctx.y} items={ctxItems} onClose={() => setCtx(null)} />
      )}
    </div>
  );
}

export { CatDots };
