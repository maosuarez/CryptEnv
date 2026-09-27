import { useState, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Icon } from './ui/Icon';
import { useVaultStore } from '../store';
import { useTranslation, type TKey } from '../i18n';

// ─── Types ────────────────────────────────────────────────────────────────────

type ImportFormat = 'env' | 'bitwarden' | '1password' | 'csv';

interface ImportItem {
  name: string;
  value?: string;
  username?: string;
  password?: string;
  url?: string;
  notes?: string;
  item_type: string;
}

interface PreviewRow extends ImportItem {
  selected: boolean;
}

interface Props {
  onClose: () => void;
}

// ─── Constants ────────────────────────────────────────────────────────────────

const FORMAT_OPTIONS: { value: ImportFormat; label: TKey; hint: TKey }[] = [
  { value: 'env',        label: 'import.formatEnv',       hint: 'import.hintEnv' },
  { value: 'bitwarden',  label: 'import.formatBitwarden', hint: 'import.hintBitwarden' },
  { value: '1password',  label: 'import.format1Password', hint: 'import.hint1Password' },
  { value: 'csv',        label: 'import.formatCsv',       hint: 'import.hintCsv' },
];

const TYPE_LABELS: Record<string, TKey> = {
  secret:     'import.typeSecret',
  credential: 'import.typeCredential',
  note:       'import.typeNote',
  link:       'import.typeLink',
};

// ─── Component ────────────────────────────────────────────────────────────────

export function ImportModal({ onClose }: Props) {
  const { t } = useTranslation();
  const showToast = useVaultStore((s) => s.showToast);

  const [format,     setFormat]     = useState<ImportFormat>('env');
  const [step,       setStep]       = useState<1 | 2 | 3>(1);
  const [rows,       setRows]       = useState<PreviewRow[]>([]);
  const [parsing,    setParsing]    = useState(false);
  const [importing,  setImporting]  = useState(false);
  const [parseError, setParseError] = useState('');

  const fileInputRef = useRef<HTMLInputElement>(null);

  // ── Step 2 → 3: read file and call vault_parse_import ─────────────────────

  const handleFileChange = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;

    setParsing(true);
    setParseError('');

    try {
      const content = await file.text();
      const items = await invoke<ImportItem[]>('vault_parse_import', {
        args: { content, format },
      });

      if (items.length === 0) {
        setParseError(t('import.errNoItems'));
        setParsing(false);
        return;
      }

      setRows(items.map((item) => ({ ...item, selected: true })));
      setStep(3);
    } catch (err: unknown) {
      setParseError(err instanceof Error ? err.message : String(err));
    } finally {
      setParsing(false);
      // Reset input so the same file can be re-selected after an error
      if (fileInputRef.current) fileInputRef.current.value = '';
    }
  };

  // ── Step 3: toggle individual row selection ────────────────────────────────

  const toggleRow = (idx: number) => {
    setRows((prev) =>
      prev.map((r, i) => (i === idx ? { ...r, selected: !r.selected } : r))
    );
  };

  const allSelected  = rows.every((r) => r.selected);
  const noneSelected = rows.every((r) => !r.selected);

  const toggleAll = () => {
    const next = !allSelected;
    setRows((prev) => prev.map((r) => ({ ...r, selected: next })));
  };

  // ── Step 3: run the import ─────────────────────────────────────────────────

  const handleImport = async () => {
    const selected = rows.filter((r) => r.selected);
    if (selected.length === 0) return;

    setImporting(true);
    try {
      // Strip the `selected` UI flag before sending to Rust
      const payload: ImportItem[] = selected.map(({ selected: _sel, ...item }) => item);
      const count = await invoke<number>('vault_import_items', { items: payload });
      const skipped = selected.length - count;

      let msg = t(count !== 1 ? 'import.imported_other' : 'import.imported_one', { count });
      if (skipped > 0) msg += t('import.skipped', { skipped });
      showToast(msg);
      onClose();
    } catch (err: unknown) {
      setParseError(err instanceof Error ? err.message : String(err));
    } finally {
      setImporting(false);
    }
  };

  // ── Render ─────────────────────────────────────────────────────────────────

  const selectedCount = rows.filter((r) => r.selected).length;

  return (
    <div className="absolute inset-0 bg-black/70 flex items-center justify-center z-20 p-4">
      <div className="w-full bg-surface border border-bd rounded-[4px] flex flex-col max-h-full overflow-hidden">

        {/* Header */}
        <div className="flex items-center gap-2 px-4 py-3 border-b border-bd shrink-0">
          <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.09em] flex-1">
            {t('import.title')}
          </div>
          <button
            onClick={onClose}
            className="text-tx3 hover:text-tx transition-colors"
            aria-label={t('common.close')}
          >
            <Icon name="close" size={13} />
          </button>
        </div>

        {/* Step indicator */}
        <div className="flex gap-px px-4 pt-3 pb-2 shrink-0">
          {([1, 2, 3] as const).map((n) => (
            <div
              key={n}
              className={[
                'flex-1 h-[3px] rounded-full transition-colors',
                step >= n ? 'bg-accent' : 'bg-bd2',
              ].join(' ')}
            />
          ))}
        </div>

        {/* Body */}
        <div className="flex-1 overflow-y-auto px-4 pb-4 min-h-0">

          {/* ── Step 1: Format selector ── */}
          {step === 1 && (
            <div>
              <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.06em] mt-2 mb-3">
                {t('import.selectFormat')}
              </div>
              <div className="flex flex-col gap-1.5">
                {FORMAT_OPTIONS.map((opt) => (
                  <label
                    key={opt.value}
                    className={[
                      'flex items-center gap-3 px-3 py-2.5 rounded-[3px] border cursor-pointer transition-colors',
                      format === opt.value
                        ? 'border-accent-d bg-accent-b'
                        : 'border-bd2 bg-raised hover:border-accent-d',
                    ].join(' ')}
                  >
                    <input
                      type="radio"
                      name="format"
                      value={opt.value}
                      checked={format === opt.value}
                      onChange={() => setFormat(opt.value)}
                      className="sr-only"
                    />
                    <span
                      className={[
                        'w-3 h-3 rounded-full border-2 shrink-0 transition-colors',
                        format === opt.value ? 'border-accent bg-accent' : 'border-bd2',
                      ].join(' ')}
                    />
                    <span className="flex-1">
                      <span className="block text-[12px] font-semibold font-ui text-tx">
                        {t(opt.label)}
                      </span>
                      <span className="block text-[10px] font-mono text-tx3 mt-0.5">
                        {t(opt.hint)}
                      </span>
                    </span>
                  </label>
                ))}
              </div>
            </div>
          )}

          {/* ── Step 2: File picker ── */}
          {step === 2 && (
            <div>
              <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.06em] mt-2 mb-3">
                {t('import.selectFileHeading')}
              </div>
              <div
                className="border-2 border-dashed border-bd2 rounded-[4px] p-6 flex flex-col items-center gap-3 cursor-pointer hover:border-accent-d transition-colors"
                onClick={() => fileInputRef.current?.click()}
              >
                <Icon name="export" size={22} color="var(--color-tx3)" />
                <div className="text-center">
                  <div className="text-[12px] font-ui font-semibold text-tx">
                    {t('import.clickToSelect')}
                  </div>
                  <div className="text-[10px] font-mono text-tx3 mt-0.5">
                    {format === 'env' ? t('import.envFiles') : t('import.csvFiles')}
                  </div>
                </div>
              </div>
              <input
                ref={fileInputRef}
                type="file"
                accept={format === 'env' ? '.env,text/*' : '.csv,text/*'}
                className="sr-only"
                onChange={handleFileChange}
              />
              {parsing && (
                <div className="mt-3 flex items-center gap-2 text-[11px] font-mono text-tx3">
                  <div className="w-3 h-3 rounded-full border-2 border-transparent border-t-accent animate-spin-fast" />
                  {t('import.parsingFile')}
                </div>
              )}
              {parseError && (
                <div className="mt-3 px-3 py-2 bg-danger-b border border-danger rounded-[3px] text-danger text-[11px] font-mono">
                  {parseError}
                </div>
              )}
            </div>
          )}

          {/* ── Step 3: Preview table ── */}
          {step === 3 && (
            <div>
              <div className="flex items-center justify-between mt-2 mb-2">
                <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.06em]">
                  {t(rows.length !== 1 ? 'import.preview_other' : 'import.preview_one', { count: rows.length })}
                </div>
                <button
                  onClick={toggleAll}
                  className="text-[10px] font-mono text-tx3 hover:text-tx transition-colors"
                >
                  {allSelected ? t('import.deselectAll') : t('import.selectAll')}
                </button>
              </div>

              <div className="border border-bd rounded-[3px] overflow-hidden">
                {/* Table header */}
                <div className="grid grid-cols-[24px_1fr_80px_80px] gap-x-2 px-2.5 py-1.5 bg-raised border-b border-bd text-[9px] font-semibold text-tx3 font-mono tracking-[0.07em]">
                  <span />
                  <span>{t('import.colName')}</span>
                  <span>{t('import.colType')}</span>
                  <span>{t('import.colHasValue')}</span>
                </div>

                {/* Table rows */}
                <div className="max-h-[220px] overflow-y-auto">
                  {rows.map((row, idx) => (
                    <label
                      key={idx}
                      className={[
                        'grid grid-cols-[24px_1fr_80px_80px] gap-x-2 px-2.5 py-2 cursor-pointer transition-colors',
                        'border-b border-bd last:border-b-0',
                        row.selected ? 'bg-accent-b' : 'bg-surface hover:bg-raised',
                      ].join(' ')}
                    >
                      <input
                        type="checkbox"
                        checked={row.selected}
                        onChange={() => toggleRow(idx)}
                        className="sr-only"
                      />
                      <span
                        className={[
                          'w-3.5 h-3.5 rounded-[2px] border flex items-center justify-center shrink-0 mt-0.5 transition-colors',
                          row.selected ? 'bg-accent border-accent' : 'border-bd2',
                        ].join(' ')}
                      >
                        {row.selected && <Icon name="check" size={9} color="#020504" />}
                      </span>
                      <span className="text-[11px] font-mono text-tx truncate" title={row.name}>
                        {row.name}
                      </span>
                      <span className="text-[9px] font-mono text-tx3 tracking-[0.05em]">
                        {TYPE_LABELS[row.item_type] ? t(TYPE_LABELS[row.item_type]) : row.item_type.toUpperCase()}
                      </span>
                      <span className="text-[11px] font-mono text-tx3">
                        {(row.value || row.password) ? (
                          <Icon name="check" size={11} color="oklch(0.70 0.17 162)" />
                        ) : (
                          '—'
                        )}
                      </span>
                    </label>
                  ))}
                </div>
              </div>

              {parseError && (
                <div className="mt-3 px-3 py-2 bg-danger-b border border-danger rounded-[3px] text-danger text-[11px] font-mono">
                  {parseError}
                </div>
              )}
            </div>
          )}
        </div>

        {/* Footer buttons */}
        <div className="px-4 pb-4 pt-2 border-t border-bd shrink-0 flex gap-2">
          {step === 1 && (
            <>
              <button
                onClick={onClose}
                className="flex-1 py-[8px] rounded-[3px] text-[11px] font-bold tracking-[0.06em] font-ui cursor-pointer bg-transparent border border-bd2 text-tx2 hover:text-tx transition-colors"
              >
                {t('common.cancel')}
              </button>
              <button
                onClick={() => setStep(2)}
                className="flex-1 py-[8px] rounded-[3px] text-[11px] font-bold tracking-[0.06em] font-ui cursor-pointer bg-accent border-none text-[#020504] hover:opacity-90 transition-opacity"
              >
                {t('import.next')}
              </button>
            </>
          )}

          {step === 2 && (
            <>
              <button
                onClick={() => { setStep(1); setParseError(''); }}
                disabled={parsing}
                className="flex-1 py-[8px] rounded-[3px] text-[11px] font-bold tracking-[0.06em] font-ui cursor-pointer bg-transparent border border-bd2 text-tx2 hover:text-tx transition-colors disabled:opacity-40"
              >
                {t('import.back')}
              </button>
              <button
                onClick={() => fileInputRef.current?.click()}
                disabled={parsing}
                className="flex-1 py-[8px] rounded-[3px] text-[11px] font-bold tracking-[0.06em] font-ui cursor-pointer bg-accent border-none text-[#020504] hover:opacity-90 transition-opacity disabled:opacity-40 flex items-center justify-center gap-1.5"
              >
                {parsing ? (
                  <><div className="w-2.5 h-2.5 rounded-full border-2 border-transparent border-t-[#020504] animate-spin-fast" />{t('import.parsing')}</>
                ) : t('import.selectFile')}
              </button>
            </>
          )}

          {step === 3 && (
            <>
              <button
                onClick={() => { setStep(2); setRows([]); setParseError(''); }}
                disabled={importing}
                className="flex-1 py-[8px] rounded-[3px] text-[11px] font-bold tracking-[0.06em] font-ui cursor-pointer bg-transparent border border-bd2 text-tx2 hover:text-tx transition-colors disabled:opacity-40"
              >
                {t('import.back')}
              </button>
              <button
                onClick={handleImport}
                disabled={importing || noneSelected}
                className="flex-1 py-[8px] rounded-[3px] text-[11px] font-bold tracking-[0.06em] font-ui cursor-pointer bg-accent border-none text-[#020504] hover:opacity-90 transition-opacity disabled:opacity-40 flex items-center justify-center gap-1.5"
              >
                {importing ? (
                  <><div className="w-2.5 h-2.5 rounded-full border-2 border-transparent border-t-[#020504] animate-spin-fast" />{t('import.importing')}</>
                ) : t(selectedCount !== 1 ? 'import.importN_other' : 'import.importN_one', { count: selectedCount })}
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
