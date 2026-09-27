import { useState } from 'react';
import { CopyBtn } from '../ui/CopyBtn';
import { KebabBtn } from '../ui/KebabBtn';
import { ContextMenu } from '../ui/ContextMenu';
import { CatBadges } from './SecretRow';
import { useVaultStore } from '../../store';
import { useTranslation } from '../../i18n';
import type { LinkItem, Category } from '../../types';

interface Props {
  item:      LinkItem;
  cats:      Category[];
  selected?: boolean;
  onToggle?: (id: number) => void;
  onShare?:  (id: number) => void;
  onSelect?: (id: number) => void;
}

export function LinkRow({ item, cats, selected, onToggle, onShare, onSelect }: Props) {
  const [ctx, setCtx] = useState<{ x: number; y: number } | null>(null);
  const go            = useVaultStore((s) => s.go);
  const setEditTarget = useVaultStore((s) => s.setEditTarget);
  const deleteItem    = useVaultStore((s) => s.deleteItem);
  const { t }         = useTranslation();

  const short = item.url.replace(/https?:\/\//, '').replace(/\/$/, '').slice(0, 48);

  const kebab = [
    { icon: 'edit',     label: t('common.edit'),onClick: () => { setEditTarget(item); go('edit'); } },
    { icon: 'external', label: t('rows.openInBrowser'),  onClick: () => {} },
    { divider: true },
    { icon: 'trash',    label: t('common.deleteLower'), danger: true, onClick: () => deleteItem(item.id) },
  ];

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
          <span className="w-2 h-2 rounded-full shrink-0 bg-lnk" />
        )}
        <span className="flex-1 text-[13px] font-semibold text-tx overflow-hidden text-ellipsis whitespace-nowrap">
          {item.title}
        </span>
        <CatBadges names={item.categories} cats={cats} />
        <CopyBtn value={item.url} label="URL" />
        <KebabBtn menuItems={kebab} />
      </div>
      <div className="pl-5 text-xs text-tx3 font-mono overflow-hidden text-ellipsis whitespace-nowrap mb-1.5">
        {short}
      </div>
      {item.description && (
        <div className="pl-5 text-xs text-tx2 overflow-hidden text-ellipsis whitespace-nowrap">
          {item.description}
        </div>
      )}
      {ctx && <ContextMenu x={ctx.x} y={ctx.y} items={ctxItems} onClose={() => setCtx(null)} />}
    </div>
  );
}
