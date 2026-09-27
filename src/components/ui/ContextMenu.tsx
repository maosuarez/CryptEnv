import { useEffect } from 'react';
import { Icon } from './Icon';
import type { ContextMenuItemDef } from '../../types';

interface ContextMenuProps {
  x:       number;
  y:       number;
  items:   ContextMenuItemDef[];
  onClose: () => void;
}

export function ContextMenu({ x, y, items, onClose }: ContextMenuProps) {
  useEffect(() => {
    const h = () => onClose();
    const t = setTimeout(() => window.addEventListener('click', h), 10);
    return () => { clearTimeout(t); window.removeEventListener('click', h); };
  }, [onClose]);

  const menuWidth = 170;
  const itemHeight = 33;
  const menuHeight = items.filter((i) => !i.divider).length * itemHeight + 16;
  const padding = 8;

  const ax = Math.max(padding, Math.min(x - 160, window.innerWidth - menuWidth - padding));
  const ay = Math.max(padding, Math.min(y + 4, window.innerHeight - menuHeight - padding));

  return (
    <div
      onClick={(e) => e.stopPropagation()}
      style={{ top: ay, left: ax }}
      className={[
        'fixed z-[9999] bg-raised border border-bd2 rounded-[4px] overflow-hidden',
        'shadow-[0_8px_32px_rgba(0,0,0,.7)] min-w-[170px]',
        'animate-fade-in',
      ].join(' ')}
    >
      {items.map((item, i) =>
        item.divider ? (
          <div key={i} className="h-px bg-bd my-[3px]" />
        ) : (
          <button
            key={i}
            onClick={() => { item.onClick?.(); onClose(); }}
            className={[
              'flex items-center gap-2 w-full px-3 py-[7px] bg-transparent',
              'border-none text-[12px] cursor-pointer font-ui text-left whitespace-nowrap',
              'transition-colors duration-100',
              item.danger ? 'text-danger hover:bg-bd' : 'text-tx hover:bg-bd',
            ].join(' ')}
          >
            {item.icon && (
              <Icon
                name={item.icon as any}
                size={12}
                color={item.danger ? 'oklch(0.62 0.20 22)' : 'var(--color-tx2)'}
              />
            )}
            <span className="flex-1">{item.label}</span>
            {item.sub && (
              <span className="text-tx3 text-[10px] font-mono">{item.sub}</span>
            )}
          </button>
        )
      )}
    </div>
  );
}
