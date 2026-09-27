import { useState } from 'react';
import { CopyBtn } from '../ui/CopyBtn';
import { KebabBtn } from '../ui/KebabBtn';
import { CmdHL } from '../ui/CmdHL';
import { Icon } from '../ui/Icon';
import { ContextMenu } from '../ui/ContextMenu';
import { CatBadges } from './SecretRow';
import { useVaultStore } from '../../store';
import { useTranslation } from '../../i18n';
import type { CommandItem, Category } from '../../types';

interface Props {
  item:      CommandItem;
  cats:      Category[];
  selected?: boolean;
  onToggle?: (id: number) => void;
  onShare?:  (id: number) => void;
  onSelect?: (id: number) => void;
}

export function CommandRow({ item, cats, selected, onToggle, onShare, onSelect }: Props) {
  const [ctx, setCtx] = useState<{ x: number; y: number } | null>(null);
  const go             = useVaultStore((s) => s.go);
  const setEditTarget  = useVaultStore((s) => s.setEditTarget);
  const deleteItem     = useVaultStore((s) => s.deleteItem);
  const setPlaceholder = useVaultStore((s) => s.setPlaceholder);
  const { t }          = useTranslation();

  const hasPlaceholders = /\{\{/.test(item.command);

  const kebab = [
    { icon: 'edit',     label: t('common.edit'),onClick: () => { setEditTarget(item); go('edit'); } },
    ...(hasPlaceholders
      ? [{ icon: 'terminal', label: t('rows.fillPlaceholders'), onClick: () => setPlaceholder(item) }]
      : []),
    { divider: true },
    { icon: 'trash',    label: t('common.deleteLower'), danger: true,   onClick: () => deleteItem(item.id) },
  ];

  const truncated = item.command.slice(0, 70) + (item.command.length > 70 ? '…' : '');

  const ctxItems = [
    { icon: 'export', label: t('rows.shareItem'),    onClick: () => onShare?.(item.id) },
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
      {/* Header row */}
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
          <span className="w-2 h-2 rounded-full shrink-0 bg-warn" />
        )}
        <span className="flex-1 text-[13px] font-semibold text-tx overflow-hidden text-ellipsis whitespace-nowrap">
          {item.name}
        </span>
        <span className="text-[0.65rem] px-1.5 py-0.5 border border-bd rounded-[2px] text-tx3 font-mono tracking-wide shrink-0">
          {item.shell}
        </span>
        <CatBadges names={item.categories} cats={cats} />
        <KebabBtn menuItems={kebab} />
      </div>
      {/* Command preview */}
      <div className="pl-5 mb-1.5 text-xs font-mono overflow-hidden text-ellipsis whitespace-nowrap text-tx2">
        <span className="text-tx3 mr-1.5">$</span>
        <CmdHL cmd={truncated} />
      </div>
      {/* Actions row */}
      <div className="pl-5 flex items-center gap-2">
        {item.description && (
          <span className="flex-1 text-xs text-tx3 overflow-hidden text-ellipsis whitespace-nowrap italic">
            {item.description}
          </span>
        )}
        <CopyBtn value={item.command} label={hasPlaceholders ? t('rows.copyRaw') : t('rows.copy')} />
        {hasPlaceholders && (
          <button
            onClick={() => setPlaceholder(item)}
            className={[
              'bg-warn-b border border-warn-d rounded-[3px] text-warn cursor-pointer',
              'px-2 py-1 text-xs font-medium tracking-wide font-ui whitespace-nowrap',
              'flex items-center gap-2 transition-all duration-150 hover:opacity-80',
            ].join(' ')}
          >
            <Icon name="terminal" size={12} color="oklch(0.72 0.16 68)" />
            {t('rows.fill')}
          </button>
        )}
      </div>
      {ctx && <ContextMenu x={ctx.x} y={ctx.y} items={ctxItems} onClose={() => setCtx(null)} />}
    </div>
  );
}
