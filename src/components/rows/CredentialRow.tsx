import { useState } from 'react';
import { Icon } from '../ui/Icon';
import { CopyBtn } from '../ui/CopyBtn';
import { KebabBtn } from '../ui/KebabBtn';
import { ContextMenu } from '../ui/ContextMenu';
import { CatBadges } from './SecretRow';
import { useVaultStore } from '../../store';
import { useTranslation } from '../../i18n';
import type { CredentialItem, Category } from '../../types';

interface Props {
  item:      CredentialItem;
  cats:      Category[];
  selected?: boolean;
  onToggle?: (id: number) => void;
  onShare?:  (id: number) => void;
  onSelect?: (id: number) => void;
}

export function CredentialRow({ item, cats, selected, onToggle, onShare, onSelect }: Props) {
  const [showPw, setShowPw] = useState(false);
  const [ctx, setCtx] = useState<{ x: number; y: number } | null>(null);
  const go            = useVaultStore((s) => s.go);
  const setEditTarget = useVaultStore((s) => s.setEditTarget);
  const deleteItem    = useVaultStore((s) => s.deleteItem);
  const { t }         = useTranslation();

  const domain = item.url?.replace(/https?:\/\//, '').split('/')[0] ?? '';

  const kebab = [
    { icon: 'edit',     label: t('common.edit'),onClick: () => { setEditTarget(item); go('edit'); } },
    { icon: 'external', label: t('rows.openUrl'),      onClick: () => {} },
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
          <span className="w-2 h-2 rounded-full shrink-0 bg-cred" />
        )}
        <span className="flex-1 text-[13px] font-semibold text-tx overflow-hidden text-ellipsis whitespace-nowrap">
          {item.name}
        </span>
        <CatBadges names={item.categories} cats={cats} />
        <KebabBtn menuItems={kebab} />
      </div>
      {domain && (
        <div className="pl-5 text-xs text-tx3 font-mono mb-2 overflow-hidden text-ellipsis whitespace-nowrap">
          {domain}
        </div>
      )}
      <div className="pl-5 flex flex-col gap-2">
        {/* Username */}
        <div className="flex items-center gap-2">
          <Icon name="person" size={12} color="var(--color-tx4)" />
          <span className="flex-1 text-xs font-mono text-tx2 overflow-hidden text-ellipsis whitespace-nowrap">
            {item.username}
          </span>
          <CopyBtn value={item.username} label={t('rows.user')} title={t('rows.copyUsername')} />
        </div>
        {/* Password */}
        <div className="flex items-center gap-2">
          <Icon name="key" size={12} color="var(--color-tx4)" />
          <span
            className={[
              'flex-1 text-xs font-mono overflow-hidden text-ellipsis whitespace-nowrap',
              showPw ? 'text-tx tracking-[0.02em]' : 'text-tx3 tracking-[0.12em]',
            ].join(' ')}
          >
            {showPw ? item.password : '••••••••••••'}
          </span>
          <button
            onClick={(e) => { e.stopPropagation(); setShowPw((v) => !v); }}
            aria-label={showPw ? t('common.hide') : t('common.show')}
            className="bg-transparent border-none cursor-pointer text-tx3 flex p-0.5 hover:text-tx transition-colors"
          >
            <Icon name={showPw ? 'eyeOff' : 'eye'} size={13} />
          </button>
          <CopyBtn value={item.password} label={t('rows.pass')} title={t('rows.copyPassword')} />
        </div>
      </div>
      {item.notes && (
        <div className="mt-2 pl-5 text-xs text-tx3 overflow-hidden text-ellipsis whitespace-nowrap italic">
          {item.notes}
        </div>
      )}
      {ctx && <ContextMenu x={ctx.x} y={ctx.y} items={ctxItems} onClose={() => setCtx(null)} />}
    </div>
  );
}
