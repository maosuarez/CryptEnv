import type { Category } from '../../types';
import { useTranslation } from '../../i18n';

interface TagInputProps {
  selected:   string[];
  categories: Category[];
  onChange:   (selected: string[]) => void;
}

export function TagInput({ selected, categories, onChange }: TagInputProps) {
  const avail = categories.filter((c) => !selected.includes(c.name));
  const { t } = useTranslation();

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
    </div>
  );
}
