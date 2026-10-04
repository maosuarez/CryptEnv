import { useState, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Icon } from './ui/Icon';
import { useVaultStore, isVaultLockedError } from '../store';
import type { VaultItem, Category } from '../types';
import { useTranslation } from '../i18n';

interface BackupModalProps {
  onClose: () => void;
}

type Tab = 'export' | 'restore';

/** Mirrors `vault::backup::RestoreSummary` (camelCase from the backend). */
interface RestoreSummary {
  items: number;
  categories: number;
  projects: number;
  environments: number;
  backupVersion: number;
  projectsIncluded: boolean;
  holdingProject: string | null;
}

function PwField({
  label,
  value,
  show,
  onChange,
  onToggle,
}: {
  label: string;
  value: string;
  show: boolean;
  onChange: (v: string) => void;
  onToggle: () => void;
}) {
  return (
    <div className="mb-3">
      <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.06em] mb-1">
        {label}
      </div>
      <div className="relative">
        <input
          type={show ? 'text' : 'password'}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          autoComplete="off"
          className="w-full bg-bg border border-bd2 text-tx font-mono text-[13px] rounded-[3px] px-3 py-[7px] pr-9 outline-none focus:border-accent-d transition-colors"
        />
        <button
          type="button"
          onClick={onToggle}
          className="absolute right-2.5 top-1/2 -translate-y-1/2 text-tx3 hover:text-tx transition-colors"
        >
          <Icon name={show ? 'eyeOff' : 'eye'} size={14} />
        </button>
      </div>
    </div>
  );
}

export function BackupModal({ onClose }: BackupModalProps) {
  const { t } = useTranslation();
  const showToast = useVaultStore((s) => s.showToast);
  const unlockWithPayload = useVaultStore((s) => s.unlockWithPayload);
  const lockedByBackend = useVaultStore((s) => s.lockedByBackend);
  const [tab, setTab] = useState<Tab>('export');

  // ── Export state ────────────────────────────────────────────────────────────
  const [exportPath, setExportPath] = useState('');
  const [exporting, setExporting]   = useState(false);
  const [exportMsg, setExportMsg]   = useState('');
  const [exportErr, setExportErr]   = useState('');

  // ── Restore state ───────────────────────────────────────────────────────────
  const [restoreMode, setRestoreMode]   = useState<'merge' | 'replace'>('merge');
  const [restorePw,   setRestorePw]     = useState('');
  const [showPw,      setShowPw]        = useState(false);
  const [currentPw,   setCurrentPw]     = useState('');
  const [showCurrentPw, setShowCurrentPw] = useState(false);
  const [summary,     setSummary]       = useState<RestoreSummary | null>(null);
  const [fileContent, setFileContent]   = useState<string | null>(null);
  const [fileName,    setFileName]      = useState('');
  const [restoring,   setRestoring]     = useState(false);
  const [restoreErr,  setRestoreErr]    = useState('');
  const fileInputRef = useRef<HTMLInputElement>(null);

  // ── Export handler ──────────────────────────────────────────────────────────
  const handleExport = async () => {
    const path = exportPath.trim();
    if (!path) {
      setExportErr(t('backup.errNoPath'));
      return;
    }
    // Append .cenvbak extension if missing
    const finalPath = path.endsWith('.cenvbak') ? path : `${path}.cenvbak`;
    setExporting(true);
    setExportErr('');
    setExportMsg('');
    try {
      const count = await invoke<number>('vault_export_backup', { path: finalPath });
      setExportMsg(t(count !== 1 ? 'backup.exportedTo_other' : 'backup.exportedTo_one', { count, path: finalPath }));
      showToast(t('backup.toastSaved', { count }));
    } catch (e: unknown) {
      setExportErr(e instanceof Error ? e.message : String(e));
    } finally {
      setExporting(false);
    }
  };

  // ── File picker handler ─────────────────────────────────────────────────────
  const handleFileChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    setFileName(file.name);
    setFileContent(null);
    setRestoreErr('');
    const reader = new FileReader();
    reader.onload = (ev) => {
      setFileContent(ev.target?.result as string ?? null);
    };
    reader.onerror = () => setRestoreErr(t('backup.errReadFile'));
    reader.readAsText(file);
  };

  // ── Restore handler ─────────────────────────────────────────────────────────
  const handleRestore = async () => {
    if (!fileContent) {
      setRestoreErr(t('backup.errNoFile'));
      return;
    }
    if (!restorePw) {
      setRestoreErr(t('backup.errNoPassword'));
      return;
    }
    if (restoreMode === 'replace' && !currentPw) {
      setRestoreErr(t('backup.errNoCurrentPassword'));
      return;
    }
    setRestoring(true);
    setRestoreErr('');
    try {
      const result = await invoke<RestoreSummary>('vault_import_backup_data', {
        data: fileContent,
        masterPassword: restorePw,
        merge: restoreMode === 'merge',
        currentPassword: restoreMode === 'replace' ? currentPw : null,
      });
      if (restoreMode === 'replace') {
        // The vault key is now the backup's: reload what the UI shows.
        const payload = await invoke<{ items: VaultItem[]; categories: Category[] }>(
          'vault_unlock',
          { password: restorePw },
        );
        await unlockWithPayload(payload);
      }
      showToast(
        t(restoreMode === 'merge' ? 'backup.toastMergedFull' : 'backup.toastRestoredFull', {
          items: result.items,
          projects: result.projects,
          environments: result.environments,
        }),
      );
      setCurrentPw('');
      setRestorePw('');
      if (result.projectsIncluded) {
        onClose();
      } else {
        setSummary(result);
      }
    } catch (e: unknown) {
      if (isVaultLockedError(e)) {
        lockedByBackend();
        return;
      }
      setRestoreErr(e instanceof Error ? e.message : String(e));
    } finally {
      setRestoring(false);
    }
  };

  const tabCls = (target: Tab) =>
    [
      'flex-1 py-[7px] text-[11px] font-bold tracking-[0.06em] font-ui cursor-pointer transition-colors',
      'border-b-2 rounded-none bg-transparent',
      tab === target
        ? 'border-accent text-accent'
        : 'border-transparent text-tx3 hover:text-tx',
    ].join(' ');

  return (
    <div className="absolute inset-0 bg-black/70 flex items-center justify-center z-20 p-5">
      <div className="w-full bg-surface border border-bd rounded-[4px] flex flex-col">
        {/* Header */}
        <div className="flex items-center px-4 pt-3 pb-0 border-b border-bd">
          <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.09em] flex-1">
            {t('backup.title')}
          </div>
          <button
            onClick={onClose}
            className="text-tx3 hover:text-tx transition-colors"
          >
            <Icon name="close" size={13} />
          </button>
        </div>

        {/* Tabs */}
        <div className="flex border-b border-bd px-4">
          <button className={tabCls('export')} onClick={() => setTab('export')}>
            {t('backup.tabExport')}
          </button>
          <button className={tabCls('restore')} onClick={() => setTab('restore')}>
            {t('backup.tabRestore')}
          </button>
        </div>

        {/* Body */}
        <div className="px-4 py-4 flex flex-col gap-3">
          {tab === 'export' && (
            <>
              <p className="text-[11px] text-tx3 font-mono leading-[1.7]">
                {t('backup.exportIntroBefore')}<code>.cenvbak</code>{t('backup.exportIntroAfter')}
              </p>

              <div>
                <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.06em] mb-1">
                  {t('backup.savePath')}
                </div>
                <input
                  type="text"
                  value={exportPath}
                  onChange={(e) => { setExportPath(e.target.value); setExportErr(''); setExportMsg(''); }}
                  placeholder="C:\Users\you\Desktop\vault-backup"
                  className="w-full bg-bg border border-bd2 text-tx font-mono text-[12px] rounded-[3px] px-3 py-[7px] outline-none focus:border-accent-d transition-colors placeholder:text-tx3"
                />
                <div className="text-[10px] text-tx3 font-mono mt-1">
                  {t('backup.extensionBefore')}<code>.cenvbak</code>{t('backup.extensionAfter')}
                </div>
              </div>

              {exportErr && (
                <div className="px-3 py-2 bg-danger-b border border-danger rounded-[3px] text-danger text-[11px] font-mono">
                  {exportErr}
                </div>
              )}
              {exportMsg && (
                <div className="px-3 py-2 bg-raised border border-bd rounded-[3px] text-accent text-[11px] font-mono flex items-center gap-2">
                  <Icon name="check" size={12} color="currentColor" />
                  {exportMsg}
                </div>
              )}

              <button
                onClick={handleExport}
                disabled={exporting}
                className="w-full py-[9px] rounded-[3px] text-[12px] font-bold tracking-[0.06em] font-ui cursor-pointer bg-accent border-none text-[#020504] hover:opacity-90 transition-opacity disabled:opacity-40 flex items-center justify-center gap-1.5 mt-1"
              >
                {exporting ? (
                  <>
                    <div className="w-3 h-3 rounded-full border-2 border-transparent border-t-[#020504] animate-spin-fast" />
                    {t('backup.exporting')}
                  </>
                ) : (
                  <>
                    <Icon name="export" size={12} color="#020504" />
                    {t('backup.exportBackup')}
                  </>
                )}
              </button>
            </>
          )}

          {tab === 'restore' && summary && (
            <>
              <div className="px-3 py-2 bg-raised border border-bd rounded-[3px] text-accent text-[11px] font-mono leading-[1.7]">
                {t('backup.toastRestoredFull', {
                  items: summary.items,
                  projects: summary.projects,
                  environments: summary.environments,
                })}
              </div>
              <p className="text-[11px] text-tx3 font-mono leading-[1.7]">
                {t('backup.legacyNote', { name: summary.holdingProject ?? '' })}
              </p>
              <button
                onClick={onClose}
                className="w-full py-[8px] rounded-[3px] text-[11px] font-bold tracking-[0.06em] font-ui cursor-pointer bg-accent border-none text-[#020504] hover:opacity-90 transition-opacity"
              >
                {t('common.done')}
              </button>
            </>
          )}

          {tab === 'restore' && !summary && (
            <>
              {/* Mode selection */}
              <div className="flex gap-2">
                {(['merge', 'replace'] as const).map((mode) => (
                  <button
                    key={mode}
                    onClick={() => setRestoreMode(mode)}
                    className={[
                      'flex-1 py-[6px] text-[11px] font-bold tracking-[0.05em] font-ui rounded-[3px] border cursor-pointer transition-all',
                      restoreMode === mode
                        ? 'bg-accent-b border-accent-d text-accent'
                        : 'bg-transparent border-bd2 text-tx3 hover:text-tx',
                    ].join(' ')}
                  >
                    {mode === 'merge' ? t('backup.merge') : t('backup.replace')}
                  </button>
                ))}
              </div>
              <p className="text-[10px] text-tx3 font-mono leading-[1.6] -mt-1">
                {restoreMode === 'merge'
                  ? t('backup.mergeHint')
                  : t('backup.replaceHint')}
              </p>

              {/* File picker */}
              <div>
                <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.06em] mb-1">
                  {t('backup.backupFile')}
                </div>
                <input
                  ref={fileInputRef}
                  type="file"
                  accept=".cenvbak"
                  onChange={handleFileChange}
                  className="hidden"
                />
                <button
                  onClick={() => fileInputRef.current?.click()}
                  className="w-full py-[7px] px-3 rounded-[3px] border border-bd2 bg-bg text-left text-[12px] font-mono text-tx2 hover:border-accent-d transition-colors cursor-pointer flex items-center gap-2"
                >
                  <Icon name="export" size={12} />
                  <span className="flex-1 truncate">
                    {fileName || t('backup.chooseFile')}
                  </span>
                </button>
              </div>

              {/* Password field */}
              <PwField
                label={t('backup.masterPassword')}
                value={restorePw}
                show={showPw}
                onChange={(v) => { setRestorePw(v); setRestoreErr(''); }}
                onToggle={() => setShowPw((v) => !v)}
              />

              {restoreMode === 'replace' && (
                <PwField
                  label={t('backup.currentPassword')}
                  value={currentPw}
                  show={showCurrentPw}
                  onChange={(v) => { setCurrentPw(v); setRestoreErr(''); }}
                  onToggle={() => setShowCurrentPw((v) => !v)}
                />
              )}

              {restoreErr && (
                <div className="px-3 py-2 bg-danger-b border border-danger rounded-[3px] text-danger text-[11px] font-mono">
                  {restoreErr}
                </div>
              )}

              <div className="flex gap-2 mt-1">
                <button
                  onClick={onClose}
                  disabled={restoring}
                  className="flex-1 py-[8px] rounded-[3px] text-[11px] font-bold tracking-[0.06em] font-ui cursor-pointer bg-transparent border border-bd2 text-tx2 hover:text-tx transition-colors disabled:opacity-40"
                >
                  {t('common.cancel')}
                </button>
                <button
                  onClick={handleRestore}
                  disabled={restoring || !fileContent || !restorePw || (restoreMode === 'replace' && !currentPw)}
                  className={[
                    'flex-1 py-[8px] rounded-[3px] text-[11px] font-bold tracking-[0.06em] font-ui cursor-pointer border-none',
                    'flex items-center justify-center gap-1.5 transition-opacity disabled:opacity-40',
                    restoreMode === 'replace'
                      ? 'bg-danger text-white hover:opacity-90'
                      : 'bg-accent text-[#020504] hover:opacity-90',
                  ].join(' ')}
                >
                  {restoring ? (
                    <>
                      <div className={`w-3 h-3 rounded-full border-2 border-transparent animate-spin-fast ${restoreMode === 'replace' ? 'border-t-white' : 'border-t-[#020504]'}`} />
                      {t('backup.restoring')}
                    </>
                  ) : restoreMode === 'replace' ? (
                    t('backup.replaceVault')
                  ) : (
                    t('backup.mergeIntoVault')
                  )}
                </button>
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
