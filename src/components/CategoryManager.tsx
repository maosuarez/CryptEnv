import { useState } from 'react';
import { Icon } from './ui/Icon';
import { useVaultStore } from '../store';
import { CAT_COLORS_PRESET } from '../store';
import type { Category } from '../types';
import { useTranslation } from '../i18n';

export function CategoryManager() {
  const cats    = useVaultStore((s) => s.cats);
  const items   = useVaultStore((s) => s.items);
  const goBack  = useVaultStore((s) => s.goBack);
  const saveCats = useVaultStore((s) => s.saveCats);
  const { t } = useTranslation();

  const [list,       setList]       = useState<Category[]>(cats.map((c) => ({ ...c })));
  const [editing,    setEditing]    = useState<string | null>(null);
  const [newName,    setNewName]    = useState('');
  const [confirmDel, setConfirmDel] = useState<Category | null>(null);
  const [saved,      setSaved]      = useState(false);

  const catCount = (name: string) => items.filter((it) => (it.categories ?? []).includes(name)).length;

  const addCat = () => {
    const n = newName.trim();
    if (!n || list.find((c) => c.name === n)) return;
    const color = CAT_COLORS_PRESET[list.length % CAT_COLORS_PRESET.length];
    setList((l) => [...l, { id: `c${Date.now()}`, name: n, color }]);
    setNewName('');
  };

  const rename = (id: string, name: string) => {
    setList((l) => l.map((c) => (c.id === id ? { ...c, name } : c)));
    setEditing(null);
  };

  const setColor = (id: string, color: string) =>
    setList((l) => l.map((c) => (c.id === id ? { ...c, color } : c)));

  const remove = (id: string) => {
    setList((l) => l.filter((c) => c.id !== id));
    setConfirmDel(null);
  };

  const handleSave = async () => {
    try {
      await saveCats(list);
      setSaved(true);
      setTimeout(() => setSaved(false), 1500);
    } catch {}
  };

  return (
    <div className="flex-1 flex flex-col overflow-hidden relative animate-fade-in">
      {/* Header */}
      <div className="px-3.5 py-[9px] border-b border-bd flex items-center gap-[10px] shrink-0">
        <button
          onClick={goBack}
          className="flex items-center gap-1 text-[12px] font-medium font-ui text-tx3 bg-transparent border-none cursor-pointer hover:text-tx transition-colors"
        >
          <Icon name="back" size={13} />{t('common.back')}
        </button>
        <div className="flex-1 text-[13px] font-semibold text-center text-tx">{t('categories.title')}</div>
        <div className="w-[50px]" />
      </div>

      {/* List */}
      <div className="flex-1 overflow-y-auto bg-surface py-2">
        {list.map((cat) => (
          <div
            key={cat.id}
            className="flex items-center gap-[10px] px-3.5 py-2 border-b border-bd hover:bg-raised transition-colors duration-100"
          >
            <Icon name="drag" size={12} color="var(--color-tx3)" />

            {/* Color picker */}
            <div className="relative shrink-0">
              <div
                className="w-[14px] h-[14px] rounded-full border border-bd2 cursor-pointer"
                style={{ background: cat.color }}
                onClick={() => setEditing(editing === `color-${cat.id}` ? null : `color-${cat.id}`)}
              />
              {editing === `color-${cat.id}` && (
                <div
                  className="fixed z-[9999] bg-raised border border-bd2 rounded-[4px] p-[7px] flex gap-[5px] flex-wrap w-[120px] shadow-[0_6px_20px_rgba(0,0,0,.7)]"
                  onClick={(e) => e.stopPropagation()}
                >
                  {CAT_COLORS_PRESET.map((c) => (
                    <div
                      key={c}
                      onClick={() => { setColor(cat.id, c); setEditing(null); }}
                      className="w-4 h-4 rounded-full cursor-pointer"
                      style={{
                        background: c,
                        border: cat.color === c ? '2px solid var(--color-tx)' : '2px solid transparent',
                      }}
                    />
                  ))}
                </div>
              )}
            </div>

            {/* Name */}
            {editing === cat.id ? (
              <input
                autoFocus
                defaultValue={cat.name}
                onBlur={(e) => rename(cat.id, e.target.value || cat.name)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') rename(cat.id, (e.target as HTMLInputElement).value || cat.name);
                  if (e.key === 'Escape') setEditing(null);
                }}
                className="flex-1 text-[12px] font-ui px-[5px] py-[2px] bg-bg border border-accent-d rounded-[2px] text-tx outline-none"
              />
            ) : (
              <span className="flex-1 text-[12px] font-medium text-tx">{cat.name}</span>
            )}

            <span className="text-[11px] text-tx2 font-mono min-w-5 text-right">{catCount(cat.name)}</span>

            <button
              onClick={() => setEditing(editing === cat.id ? null : cat.id)}
              aria-label={t('categories.rename')}
              title={t('categories.rename')}
              className="bg-transparent border-none cursor-pointer text-tx3 flex hover:text-tx transition-colors"
            >
              <Icon name="edit" size={12} />
            </button>
            <button
              onClick={() => setConfirmDel(cat)}
              aria-label={t('common.deleteLower')}
              title={t('common.deleteLower')}
              className="bg-transparent border-none cursor-pointer text-tx3 flex hover:text-danger transition-colors"
            >
              <Icon name="trash" size={12} />
            </button>
          </div>
        ))}

        {/* Add new */}
        <div className="px-3.5 py-[10px] flex gap-[7px] items-center">
          <input
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && addCat()}
            placeholder={t('categories.newPlaceholder')}
            className="flex-1 px-[9px] py-[7px] bg-raised border border-bd2 rounded-[3px] text-[12px] text-tx outline-none focus:border-accent-d transition-colors"
          />
          <button
            onClick={addCat}
            className="flex items-center gap-1 bg-accent border-none rounded-[3px] px-3 py-[7px] text-[11px] font-bold cursor-pointer font-ui text-[#020504] shrink-0 hover:opacity-90 transition-opacity"
          >
            <Icon name="plus" size={12} color="#020504" />
            {t('categories.add')}
          </button>
        </div>
      </div>

      {/* Footer */}
      <div className="px-3.5 py-[10px] border-t border-bd shrink-0 bg-bg">
        <button
          onClick={handleSave}
          className={[
            'w-full py-[9px] rounded-[3px] text-[12px] font-bold tracking-[0.06em] cursor-pointer font-ui',
            'flex items-center justify-center gap-1.5 transition-all duration-200',
            saved
              ? 'bg-accent-b border border-accent-d text-accent'
              : 'bg-accent border-none text-[#020504] hover:opacity-90',
          ].join(' ')}
        >
          {saved ? (
            <><Icon name="check" size={12} color="oklch(0.70 0.17 162)" />{t('common.saved')}</>
          ) : (
            t('categories.save')
          )}
        </button>
      </div>

      {/* Delete confirm overlay */}
      {confirmDel && (
        <div className="absolute inset-0 bg-[rgba(10,11,14,.85)] flex items-center justify-center p-6 z-[100] backdrop-blur-[4px]">
          <div className="bg-surface border border-danger rounded-[4px] p-[22px] w-full">
            <div className="text-[13px] font-bold mb-1.5 text-tx">{t('categories.deleteTitle', { name: confirmDel.name })}</div>
            <div className="text-[12px] text-tx2 mb-[18px] leading-[1.5]">
              {t('categories.deleteBody', { n: catCount(confirmDel.name) })}
            </div>
            <div className="flex gap-2">
              <button
                onClick={() => setConfirmDel(null)}
                className="flex-1 py-2 bg-transparent border border-bd2 rounded-[3px] text-tx2 text-[12px] cursor-pointer font-ui"
              >
                {t('common.cancel')}
              </button>
              <button
                onClick={() => remove(confirmDel.id)}
                className="flex-1 py-2 bg-danger border-none rounded-[3px] text-white text-[12px] font-bold cursor-pointer font-ui"
              >
                {t('common.delete')}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
