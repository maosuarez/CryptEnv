import { useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Icon } from './ui/Icon';
import { RelayCodeDisplay } from './ui/RelayCodeDisplay';
import { useTranslation } from '../i18n';
import type { Project, VaultItem } from '../types';

// ─── Whole-project relay sharing (issue #4) ───────────────────────────────────
// "SHARE PROJECT" / "RECEIVE PROJECT" — carries structure (environment names,
// isDefault) *and* decrypted values for the selected environments, in one
// relay round-trip. See docs/plans/issue-4-share-whole-project-via-relay.md.
//
// The sender always sees, per selected environment, the KEY -> item name
// pairs that are about to leave the machine (D4) -- built entirely from data
// this component already receives as props (project + the full vault item
// list), so no new backend command or decryption pass is needed for the
// manifest itself.

export type ProjectShareMode = 'send' | 'receive';

export interface ProjectShareModalProps {
  mode: ProjectShareMode;
  /** Required when mode === 'send'. */
  project?: Project;
  /** Full vault item list (metadata only -- names, never values), used to
   *  resolve `EnvironmentVar.itemId` -> a human-readable name for the manifest. */
  items: VaultItem[];
  onClose: () => void;
  /** Called after a successful receive so the caller can refresh projects + vault items. */
  onReceived?: () => void;
}

interface SendResult {
  code: string;
  passphrase: string;
  project: string;
  environmentCount: number;
  itemCount: number;
}

interface ReceiveResult {
  project: string;
  environments: string[];
  itemCount: number;
}

const PROD_NAME_PATTERN = ['prod', 'production', 'live', 'release'];

function looksProduction(name: string): boolean {
  const lower = name.toLowerCase();
  return PROD_NAME_PATTERN.some((p) => lower.includes(p));
}

function displayName(item: VaultItem | undefined, itemId: number): string {
  if (!item) return `#${itemId}`;
  const name = ('name' in item ? item.name : 'title' in item ? (item as any).title : '') as string | undefined;
  return name || `#${itemId}`;
}

// ─── Small local primitives (mirrors ShareModal.tsx's button/shell styling) ──

function BtnPrimary({ children, onClick, disabled }: { children: React.ReactNode; onClick?: () => void; disabled?: boolean }) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className="flex items-center justify-center gap-2 rounded-[3px] px-4 h-9 bg-accent text-[#020504] text-[11px] font-bold tracking-wider font-ui cursor-pointer transition-opacity shrink-0 disabled:opacity-30 disabled:cursor-not-allowed hover:opacity-90"
    >
      {children}
    </button>
  );
}

function BtnSecondary({ children, onClick, disabled }: { children: React.ReactNode; onClick?: () => void; disabled?: boolean }) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className="flex items-center justify-center gap-2 rounded-[3px] px-4 h-9 border border-bd2 text-tx2 text-[11px] font-medium tracking-wider font-ui bg-transparent cursor-pointer transition-all duration-150 shrink-0 disabled:opacity-30 disabled:cursor-not-allowed hover:border-tx3 hover:text-tx"
    >
      {children}
    </button>
  );
}

function InlineError({ msg }: { msg: string }) {
  return (
    <div className="text-[12px] text-danger font-mono bg-danger-b border border-danger rounded-[3px] px-3 py-2 mb-3">
      {msg}
    </div>
  );
}

export function ProjectShareModal({ mode, project, items, onClose, onReceived }: ProjectShareModalProps) {
  const { t } = useTranslation();
  const itemsById = useMemo(() => new Map(items.map((i) => [i.id, i])), [items]);

  // ── Send state ──
  const [selectedEnvIds, setSelectedEnvIds] = useState<Set<number>>(() => {
    if (!project || project.environments.length === 0) return new Set();
    const def = project.environments.find((e) => e.isDefault);
    return new Set([def ? def.id : project.environments[0].id]);
  });
  const [sending, setSending] = useState(false);
  const [sendResult, setSendResult] = useState<SendResult | null>(null);

  // ── Receive state ──
  const [code, setCode] = useState('');
  const [passphrase, setPassphrase] = useState('');
  const [overrideName, setOverrideName] = useState('');
  const [showRename, setShowRename] = useState(false);
  const [receiving, setReceiving] = useState(false);
  const [receiveResult, setReceiveResult] = useState<ReceiveResult | null>(null);

  const [error, setError] = useState('');

  const toggleEnv = (id: number) => {
    setSelectedEnvIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  };

  const handleSend = async () => {
    if (!project || selectedEnvIds.size === 0) return;
    setSending(true);
    setError('');
    try {
      const result = await invoke<SendResult>('project_relay_send', {
        projectId: project.id,
        environmentIds: Array.from(selectedEnvIds),
      });
      setSendResult(result);
    } catch (e) {
      setError(String(e));
    } finally {
      setSending(false);
    }
  };

  const handleReceive = async () => {
    if (!code || !passphrase) return;
    setReceiving(true);
    setError('');
    try {
      const result = await invoke<ReceiveResult>('project_relay_receive', {
        code: code.toUpperCase(),
        passphrase,
        projectNameOverride: overrideName.trim() || null,
      });
      setReceiveResult(result);
      onReceived?.();
    } catch (e) {
      const msg = String(e);
      if (msg.includes('conflict:')) {
        setShowRename(true);
        const match = msg.match(/'([^']+)'/);
        if (match && !overrideName) setOverrideName(`${match[1]}-received`);
        setError(t('projectShare.conflict'));
      } else {
        setError(msg);
      }
    } finally {
      setReceiving(false);
    }
  };

  function renderSend() {
    if (!project) return null;

    if (sendResult) {
      return (
        <>
          <div className="flex items-center gap-2 mb-4">
            <div className="w-7 h-7 rounded-full bg-accent-b border border-accent-d flex items-center justify-center shrink-0">
              <Icon name="check" size={13} color="oklch(0.70 0.17 162)" />
            </div>
            <div className="text-[13px] font-semibold text-tx">{t('projectShare.uploaded')}</div>
          </div>
          <div className="text-[11px] text-tx3 font-mono mb-3">
            {t(sendResult.environmentCount === 1 ? 'projectShare.envCount_one' : 'projectShare.envCount_other', { n: sendResult.environmentCount })}, {t(sendResult.itemCount === 1 ? 'projectShare.itemCount_one' : 'projectShare.itemCount_other', { n: sendResult.itemCount })}
          </div>
          <RelayCodeDisplay code={sendResult.code} passphrase={sendResult.passphrase} />
          <div className="flex justify-end pt-3 border-t border-bd">
            <BtnPrimary onClick={onClose}>{t('common.done')}</BtnPrimary>
          </div>
        </>
      );
    }

    if (project.environments.length === 0) {
      return (
        <>
          <InlineError msg={t('projectShare.noEnvironments')} />
          <div className="flex justify-end">
            <BtnSecondary onClick={onClose}>{t('projectShare.close')}</BtnSecondary>
          </div>
        </>
      );
    }

    return (
      <>
        <div className="text-[11px] text-tx3 mb-3 leading-[1.5]">
          {t('projectShare.selectPrefix')}<span className="text-tx font-semibold">{project.name}</span>{t('projectShare.selectSuffix')}
          {' '}{t('projectShare.selectHint')}
        </div>

        <div className="space-y-1.5 mb-4">
          {project.environments.map((env) => {
            const checked = selectedEnvIds.has(env.id);
            const prodLike = looksProduction(env.name);
            return (
              <label
                key={env.id}
                className={[
                  'flex items-center gap-2 px-3 py-2 rounded-[3px] border cursor-pointer transition-colors',
                  prodLike ? 'border-danger bg-danger-b' : checked ? 'border-accent-d bg-accent-b' : 'border-bd2 bg-raised',
                ].join(' ')}
              >
                <input type="checkbox" checked={checked} onChange={() => toggleEnv(env.id)} className="accent-accent" />
                <span className="text-[12px] font-mono text-tx flex-1">{env.name}</span>
                {env.isDefault && <span className="text-[9px] text-tx3 font-mono tracking-wide">{t('projectShare.defaultBadge')}</span>}
                {prodLike && <span className="text-[9px] text-danger font-mono font-bold tracking-wide">{t('projectShare.prodBadge')}</span>}
              </label>
            );
          })}
        </div>

        {selectedEnvIds.size > 0 && (
          <div className="mb-4">
            <div className="text-[10px] font-mono text-tx3 tracking-[0.1em] mb-2">{t('projectShare.keysLeaving')}</div>
            <div className="bg-raised border border-bd rounded-[3px] max-h-[160px] overflow-y-auto divide-y divide-bd">
              {project.environments
                .filter((e) => selectedEnvIds.has(e.id))
                .map((env) => (
                  <div key={env.id} className="px-3 py-2">
                    <div className="text-[10px] text-tx3 font-mono mb-1 tracking-wide">{env.name.toUpperCase()}</div>
                    {env.vars.length === 0 ? (
                      <div className="text-[11px] text-tx3 font-mono italic">{t('projectShare.noVariables')}</div>
                    ) : (
                      env.vars.map((v) => (
                        <div key={v.id} className="text-[11px] font-mono text-tx2 flex items-center gap-2">
                          <span className="text-tx">{v.key}</span>
                          <span className="text-tx3">{'->'}</span>
                          <span className="truncate">{displayName(itemsById.get(v.itemId), v.itemId)}</span>
                        </div>
                      ))
                    )}
                  </div>
                ))}
            </div>
          </div>
        )}

        {error && <InlineError msg={error} />}

        <div className="flex justify-between pt-3 border-t border-bd">
          <BtnSecondary onClick={onClose}>{t('common.cancel')}</BtnSecondary>
          <BtnPrimary onClick={handleSend} disabled={sending || selectedEnvIds.size === 0}>
            {sending ? t('projectShare.uploading') : t('projectShare.shareProject')}
          </BtnPrimary>
        </div>
      </>
    );
  }

  function renderReceive() {
    if (receiveResult) {
      return (
        <>
          <div className="flex items-center gap-2 mb-4">
            <div className="w-7 h-7 rounded-full bg-accent-b border border-accent-d flex items-center justify-center shrink-0">
              <Icon name="check" size={13} color="oklch(0.70 0.17 162)" />
            </div>
            <div className="text-[13px] font-semibold text-tx">{t('projectShare.received')}</div>
          </div>
          <div className="text-[12px] text-tx2 mb-2">
            <span className="font-semibold text-tx">{receiveResult.project}</span> — {t(receiveResult.itemCount === 1 ? 'projectShare.itemCount_one' : 'projectShare.itemCount_other', { n: receiveResult.itemCount })}
          </div>
          <div className="bg-raised border border-bd rounded-[3px] divide-y divide-bd mb-3 max-h-[140px] overflow-y-auto">
            {receiveResult.environments.map((name) => (
              <div key={name} className="px-3 py-2 text-[12px] font-mono text-tx2 flex items-center gap-2">
                <span className="w-1.5 h-1.5 rounded-full bg-accent shrink-0" />
                {name}
              </div>
            ))}
          </div>
          <div className="text-[10px] text-tx3 font-mono mb-3">
            {t('projectShare.receivedNote')}
          </div>
          <div className="flex justify-end pt-3 border-t border-bd">
            <BtnPrimary onClick={onClose}>{t('common.done')}</BtnPrimary>
          </div>
        </>
      );
    }

    return (
      <>
        <div className="text-[11px] text-tx3 mb-3">
          {t('projectShare.receiveIntroPrefix')}<span className="text-tx font-semibold">{t('projectShare.receiveIntroNew')}</span>{t('projectShare.receiveIntroSuffix')}
        </div>
        <div className="mb-3">
          <label className="block text-[10px] font-mono text-tx3 tracking-[0.08em] mb-1.5">{t('projectShare.code')}</label>
          <input
            type="text"
            value={code}
            onChange={(e) => setCode(e.target.value.toUpperCase())}
            placeholder="XXXX-XXXX"
            maxLength={9}
            className="w-full h-9 bg-raised border border-bd2 rounded-[3px] px-3 text-[15px] font-mono text-accent tracking-[0.2em] placeholder:text-tx3 outline-none focus:border-accent transition-colors"
          />
        </div>
        <div className="mb-3">
          <label className="block text-[10px] font-mono text-tx3 tracking-[0.08em] mb-1.5">{t('projectShare.passphrase')}</label>
          <input
            type="text"
            value={passphrase}
            onChange={(e) => setPassphrase(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Enter' && !showRename) handleReceive(); }}
            className="w-full h-9 bg-raised border border-bd2 rounded-[3px] px-3 text-[13px] font-mono text-tx placeholder:text-tx3 outline-none focus:border-accent transition-colors"
          />
        </div>

        {showRename && (
          <div className="mb-4">
            <label className="block text-[10px] font-mono text-tx3 tracking-[0.08em] mb-1.5">{t('projectShare.newProjectName')}</label>
            <input
              type="text"
              value={overrideName}
              onChange={(e) => setOverrideName(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Enter') handleReceive(); }}
              className="w-full h-9 bg-raised border border-bd2 rounded-[3px] px-3 text-[13px] font-mono text-tx placeholder:text-tx3 outline-none focus:border-accent transition-colors"
            />
          </div>
        )}

        {error && <InlineError msg={error} />}

        <div className="flex justify-between pt-3 border-t border-bd">
          <BtnSecondary onClick={onClose}>{t('common.cancel')}</BtnSecondary>
          <BtnPrimary onClick={handleReceive} disabled={receiving || !code || !passphrase || (showRename && !overrideName.trim())}>
            {receiving ? t('projectShare.downloading') : t('projectShare.receive')}
          </BtnPrimary>
        </div>
      </>
    );
  }

  return (
    <div className="absolute inset-0 bg-black/70 flex items-center justify-center z-20 p-5">
      <div className="bg-surface border border-bd rounded-[4px] p-4 w-full max-w-md max-h-[85vh] overflow-y-auto animate-fade-in">
        <div className="flex items-center justify-between mb-4">
          <div className="flex items-center gap-2">
            <Icon name="shield" size={14} color="oklch(0.70 0.17 162)" />
            <span className="text-[12px] font-bold tracking-wider font-ui text-tx">
              {mode === 'send' ? t('projectShare.titleSend') : t('projectShare.titleReceive')}
            </span>
          </div>
          <button
            onClick={onClose}
            aria-label={t('common.close')}
            className="flex items-center justify-center w-6 h-6 rounded-[3px] text-tx3 hover:text-tx hover:bg-raised transition-all duration-150 cursor-pointer border-none bg-transparent"
          >
            <Icon name="close" size={13} />
          </button>
        </div>

        {mode === 'send' ? renderSend() : renderReceive()}
      </div>
    </div>
  );
}
