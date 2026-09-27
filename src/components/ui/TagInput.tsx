import { useState } from 'react';
import type { Category } from '../../types';
import { useTranslation } from '../../i18n';

interface TagInputProps {
  selected:   string[];
  categories: Category[];
  onChange:   (selected: string[]) => void;
  /** When set, shows an inline "new tag" input. Called with a trimmed name
   *  that matches no existing category; the caller persists it and selects
   *  it. An existing name (case-insensitive) is just selected instead. */
  onCreate?:  (name: string) => Promise<void> | void;
}

export function TagInput({ selected, categories, onChange, onCreate }: TagInputProps) {
  const avail = categories.filter((c) => !selected.includes(c.name));
  const { t } = useTranslation();
  const [draft, setDraft]       = useState('');
  const [creating, setCreating] = useState(false);

  const submitDraft = async () => {
    const name = draft.trim();
    if (!name || !onCreate || creating) return;
    const existing = categories.find((c) => c.name.toLowerCase() === name.toLowerCase());
    if (existing) {
      if (!selected.includes(existing.name)) onChange([...selected, existing.name]);
      setDraft('');
      return;
    }
    setCreating(true);
    try {
      await onCreate(name);
      setDraft('');
    } catch {
      // Keep the draft so the user can retry; the caller reports the error.
    } finally {
      setCreating(false);
    }
  };

  return (
    <div>
      <div className="flex flex-wrap gap-1 min-h-6 mb-1.5">
        {selected.length === 0 && (
          <span className="text-[11px] text-tx3 self-center">
            {t('ui.noCategoriesHint')}
          </span>
        )}
        {selected.map((name) => {
          const cat = categories.find((c) => c.name === name);
          return (
            <div
              key={name}
              className="flex items-center gap-1 bg-accent-b border border-accent-d rounded-[3px] py-[2px] pl-[7px] pr-[5px] text-[11px] text-accent"
            >
              <span
                className="w-[5px] h-[5px] rounded-full shrink-0"
                style={{ background: cat?.color ?? 'oklch(0.70 0.17 162)' }}
              />
              {name}
              <button
                onClick={() => onChange(selected.filter((s) => s !== name))}
                aria-label={t('ui.removeTag', { name })}
                className="bg-transparent border-none cursor-pointer text-accent-d p-0 leading-none text-[14px] flex items-center"
              >
                ×
              </button>
            </div>
          );
        })}
      </div>

      {avail.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {avail.map((c) => (
            <button
              key={c.id}
              onClick={() => onChange([...selected, c.name])}
              className={[
                'flex items-center gap-1 bg-raised border border-bd2',
                'rounded-[3px] py-[2px] px-2 text-[11px] text-tx2',
                'cursor-pointer font-ui transition-colors duration-100 hover:text-tx',
              ].join(' ')}
            >
              <span
                className="w-[5px] h-[5px] rounded-full shrink-0"
                style={{ background: c.color }}
              />
              {c.name}
            </button>
          ))}
        </div>
      )}

      {onCreate && (
        <div className="flex items-center gap-1 mt-1.5">
          <input
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); void submitDraft(); } }}
            placeholder={t('ui.newTagPlaceholder')}
            aria-label={t('ui.newTagPlaceholder')}
            className="flex-1 min-w-0 bg-bg border border-bd2 text-tx font-mono text-[11px] rounded-[3px] px-2 py-[3px] outline-none focus:border-accent-d transition-colors"
          />
          <button
            onClick={() => void submitDraft()}
            disabled={!draft.trim() || creating}
            className="text-[10px] font-ui font-bold text-accent border border-accent-d rounded-[3px] px-2 py-[3px] hover:bg-accent-b transition-colors disabled:opacity-40"
          >
            {t('ui.addTag')}
          </button>
        </div>
      )}
    </div>
  );
}
