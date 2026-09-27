import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { platform } from '@tauri-apps/plugin-os';
import { Icon } from './ui/Icon';
import { ImportModal } from './ImportModal';
import { BackupModal } from './BackupModal';
import { ReceiveModal } from './ReceiveModal';
import { WslIntegrationSection } from './settings/WslIntegrationSection';
import { useVaultStore } from '../store';
import type { IconName } from '../types';

function Row({ icon, label, children }: { icon: IconName; label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center gap-3 min-h-[44px] py-2 border-b border-bd">
      <span className="text-tx3 shrink-0"><Icon name={icon} size={14} /></span>
      <span className="flex-1 text-[13px] font-medium text-tx font-ui">{label}</span>
      {children}
    </div>
  );
}

function Sec({ title }: { title: string }) {
  return (
    <div className="mt-8 mb-1">
      <div className="text-[14px] font-semibold text-tx2 tracking-[0.1em] font-ui pb-2 border-b border-bd">
        {title}
      </div>
    </div>
  );
}

function PwField({
  label, value, show, onChange, onToggle,
}: { label: string; value: string; show: boolean; onChange: (v: string) => void; onToggle: () => void }) {
  return (
    <div className="mb-4">
      <div className="text-[11px] font-semibold text-tx3 font-mono tracking-[0.08em] mb-1.5">{label}</div>
      <div className="relative">
        <input
          type={show ? 'text' : 'password'}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          autoComplete="off"
          className={[
            'w-full bg-transparent border-0 border-b-2 border-bd2 text-tx font-mono text-[13px]',
            'px-0 py-2 pr-8 outline-none',
            'focus:border-accent transition-colors duration-150',
          ].join(' ')}
        />
        <button
          type="button"
          onClick={onToggle}
          className="absolute right-0 top-1/2 -translate-y-1/2 text-tx3 hover:text-tx transition-colors"
        >
          <Icon name={show ? 'eyeOff' : 'eye'} size={14} />
        </button>
      </div>
    </div>
  );
}

const RELAY_SQL = `create table if not exists relay_packages (
  id          uuid primary key default gen_random_uuid(),
  code        text not null unique,
  payload     text not null,
  expires_at  timestamptz not null default (now() + interval '24 hours'),
  retrieved   boolean not null default false
);
alter table relay_packages enable row level security;
create policy "insert" on relay_packages for insert to anon with check (true);
create policy "select" on relay_packages for select to anon using (true);
create policy "update" on relay_packages for update to anon using (true) with check (true);`;

function RelayConfigSection({ showToast }: { showToast: (msg: string, type?: 'success' | 'error') => void }) {
  const [relayMode, setRelayMode] = useState<'shared' | 'custom'>('shared');
  const [url,       setUrl]       = useState('');
  const [anonKey,   setAnonKey]   = useState('');
  const [showKey,   setShowKey]   = useState(false);
  const [sqlOpen,   setSqlOpen]   = useState(false);
  const [saving,    setSaving]    = useState(false);

  useEffect(() => {
    // Relay settings are persisted server-side; no need to load them here.
  }, []);

  const handleSaveRelay = async () => {
    if (!url.trim() || !anonKey.trim()) { showToast('Both URL and Anon Key are required', 'error'); return; }
    setSaving(true);
    try {
      await invoke('vault_save_settings', {
        autoLockTimeout: 5,
        hotkey: 'Ctrl+Alt+Z',
        relaySupabaseUrl: url.trim(),
        relaySupabaseAnonKey: anonKey.trim(),
      });
      showToast('Relay settings saved');
    } catch (e) {
      showToast(String(e), 'error');
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="mt-2 mb-2">
      {/* Content switcher */}
      <div className="flex mt-3 mb-4 border border-bd rounded-[3px] overflow-hidden">
        <button
          onClick={() => setRelayMode('shared')}
          className={[
            'flex-1 h-8 text-[12px] font-semibold tracking-[0.06em] font-ui border-none cursor-pointer transition-colors duration-150',
            relayMode === 'shared'
              ? 'bg-accent text-[#020504]'
              : 'bg-transparent text-tx3 hover:text-tx hover:bg-surface',
          ].join(' ')}
        >
          SHARED RELAY
        </button>
        <button
          onClick={() => setRelayMode('custom')}
          className={[
            'flex-1 h-8 text-[12px] font-semibold tracking-[0.06em] font-ui border-none cursor-pointer transition-colors duration-150 border-l border-bd',
            relayMode === 'custom'
              ? 'bg-accent text-[#020504]'
              : 'bg-transparent text-tx3 hover:text-tx hover:bg-surface',
          ].join(' ')}
        >
          CUSTOM RELAY
        </button>
      </div>

      {relayMode === 'shared' && (
        <div className="rounded-[3px] border border-bd bg-raised px-4 py-3 space-y-1.5">
          <div className="text-[13px] font-medium text-tx font-ui">Using developer-hosted relay</div>
          <div className="text-[12px] font-mono text-tx3 leading-[1.9] space-y-0.5">
            <div>AES-256-GCM · Argon2id KDF</div>
            <div>Burn-after-read · 24h TTL</div>
            <div>Relay cannot read your data</div>
          </div>
        </div>
      )}

      {relayMode === 'custom' && (
        <div className="space-y-4">
          <div className="flex items-start gap-2 px-3 py-2 rounded-[3px] border border-bd bg-raised">
            <span className="text-tx3 text-[13px] mt-0.5 shrink-0">ℹ</span>
            <p className="text-[12px] font-mono text-tx3 leading-[1.6]">
              Your data is encrypted before upload — the relay stores only ciphertext.
            </p>
          </div>

          <div>
            <div className="text-[11px] font-semibold text-tx3 font-mono tracking-[0.08em] mb-1.5">SUPABASE PROJECT URL</div>
            <input
              type="text"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              placeholder="https://xxxx.supabase.co"
              autoComplete="off"
              className={[
                'w-full bg-transparent border-0 border-b-2 border-bd2 text-tx font-mono text-[13px]',
                'px-0 py-2 outline-none focus:border-accent transition-colors duration-150',
              ].join(' ')}
            />
          </div>

          <div>
            <div className="text-[11px] font-semibold text-tx3 font-mono tracking-[0.08em] mb-1.5">SUPABASE ANON KEY</div>
            <div className="relative">
              <input
                type={showKey ? 'text' : 'password'}
                value={anonKey}
                onChange={(e) => setAnonKey(e.target.value)}
                placeholder="eyJ..."
                autoComplete="off"
                className={[
                  'w-full bg-transparent border-0 border-b-2 border-bd2 text-tx font-mono text-[13px]',
                  'px-0 py-2 pr-8 outline-none focus:border-accent transition-colors duration-150',
                ].join(' ')}
              />
              <button
                type="button"
                onClick={() => setShowKey((v) => !v)}
                className="absolute right-0 top-1/2 -translate-y-1/2 text-tx3 hover:text-tx transition-colors"
              >
                <Icon name={showKey ? 'eyeOff' : 'eye'} size={13} />
              </button>
            </div>
          </div>

          <div className="flex items-center gap-2 pt-1">
            <button
              onClick={handleSaveRelay}
              disabled={saving}
              className={[
                'h-8 px-4 rounded-[3px] text-[12px] font-bold tracking-[0.06em] font-ui cursor-pointer',
                'bg-accent border-none text-[#020504] hover:opacity-90 disabled:opacity-40',
                'flex items-center gap-1.5',
              ].join(' ')}
            >
              {saving
                ? <><div className="w-2.5 h-2.5 rounded-full border-2 border-transparent border-t-[#020504] animate-spin-fast" />SAVING…</>
                : 'SAVE RELAY'}
            </button>
            <button
              onClick={() => setSqlOpen((v) => !v)}
              className="h-8 px-4 text-[12px] font-semibold tracking-[0.06em] font-ui text-tx3 border border-bd2 rounded-[3px] hover:text-tx transition-colors bg-transparent cursor-pointer"
            >
              {sqlOpen ? 'HIDE SQL ▲' : 'SETUP SQL ▼'}
            </button>
          </div>

          {sqlOpen && (
            <div className="rounded-[3px] border border-bd bg-raised overflow-hidden">
              <div className="flex items-center justify-between px-3 py-2 border-b border-bd">
                <span className="text-[11px] font-mono text-tx3 tracking-[0.06em]">Run in your Supabase SQL Editor</span>
                <button
                  onClick={() => navigator.clipboard.writeText(RELAY_SQL).then(() => showToast('SQL copied'))}
                  className="text-tx3 hover:text-accent transition-colors"
                  title="Copy SQL"
                  aria-label="Copy SQL"
                >
                  <Icon name="copy" size={12} />
                </button>
              </div>
              <pre className="text-[11px] font-mono text-tx2 p-4 overflow-x-auto leading-[1.6] whitespace-pre-wrap">{RELAY_SQL}</pre>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export function Settings() {
  const go               = useVaultStore((s) => s.go);
  const showToast        = useVaultStore((s) => s.showToast);
  const storeWipe        = useVaultStore((s) => s.wipe);
  const storeLockTimeout = useVaultStore((s) => s.lockTimeout);
  const storeHotkey      = useVaultStore((s) => s.hotkey);
  const setLockTimeout   = useVaultStore((s) => s.setLockTimeout);
  const setHotkey        = useVaultStore((s) => s.setHotkey);

  const [timeoutDraft, setTimeoutDraft] = useState(storeLockTimeout);
  const [hotkeyDraft,  setHotkeyDraft]  = useState(storeHotkey);
  const [capturing,    setCapturing]    = useState(false);
  const [saving,       setSaving]       = useState(false);
  const [saved,        setSaved]        = useState(false);
  const [wipeOpen,     setWipeOpen]     = useState(false);
  const [wiping,       setWiping]       = useState(false);
  const [importOpen,   setImportOpen]   = useState(false);
  const [backupOpen,   setBackupOpen]   = useState(false);
  const [shareOpen,    setShareOpen]    = useState(false);
  const [isWindows]                     = useState(() => platform() === 'windows');

  const [mcpToken,        setMcpToken]        = useState<string | null>(null);
  const [mcpTokenVisible, setMcpTokenVisible] = useState(false);
  const [generatingMcp,   setGeneratingMcp]   = useState(false);

  const [changePwOpen, setChangePwOpen] = useState(false);
  const [currentPw,    setCurrentPw]    = useState('');
  const [newPw,        setNewPw]        = useState('');
  const [confirmPw,    setConfirmPw]    = useState('');
  const [showCurrent,  setShowCurrent]  = useState(false);
  const [showNew,      setShowNew]      = useState(false);
  const [showConfirm,  setShowConfirm]  = useState(false);
  const [pwError,      setPwError]      = useState('');
  const [pwChanging,   setPwChanging]   = useState(false);

  const [bioAvailable, setBioAvailable] = useState(false);
  const [bioEnrolled,  setBioEnrolled]  = useState(false);
  const [bioPw,        setBioPw]        = useState('');
  const [showBioPw,    setShowBioPw]    = useState(false);
  const [bioWorking,   setBioWorking]   = useState(false);

  const [updateStatus,      setUpdateStatus]      = useState<string | null>(null);
  const [availableVersion,  setAvailableVersion]  = useState<string | null>(null);
  const [checkingUpdate,    setCheckingUpdate]    = useState(false);
  const [installingUpdate,  setInstallingUpdate]  = useState(false);

  const openChangePw = () => {
    setCurrentPw(''); setNewPw(''); setConfirmPw('');
    setShowCurrent(false); setShowNew(false); setShowConfirm(false);
    setPwError('');
    setChangePwOpen(true);
  };

  const closeChangePw = () => { setChangePwOpen(false); setPwError(''); };

  const handleChangePassword = async () => {
    if (newPw.length < 8) { setPwError('New password must be at least 8 characters'); return; }
    if (newPw !== confirmPw) { setPwError('New passwords do not match'); return; }
    setPwChanging(true);
    setPwError('');
    try {
      await invoke('vault_change_password', { currentPassword: currentPw, newPassword: newPw });
      closeChangePw();
      showToast('Master password changed successfully');
    } catch (e: unknown) {
      setPwError(e instanceof Error ? e.message : String(e));
    } finally {
      setPwChanging(false);
    }
  };

  useEffect(() => {
    invoke<{ autoLockTimeout: number; hotkey: string }>('vault_get_settings')
      .then((s) => {
        setTimeoutDraft(s.autoLockTimeout);
        setHotkeyDraft(s.hotkey);
        setLockTimeout(s.autoLockTimeout);
        setHotkey(s.hotkey);
      })
      .catch(() => {});

    invoke<string | null>('vault_get_mcp_token')
      .then((t) => setMcpToken(t))
      .catch(() => {});

    invoke<string>('biometric_check').then((status) => {
      if (status === 'available') {
        setBioAvailable(true);
        invoke<boolean>('biometric_is_enrolled').then(setBioEnrolled).catch(() => {});
      }
    }).catch(() => {});
  }, []);

  const handleSave = async () => {
    setSaving(true);
    try {
      await invoke('vault_save_settings', {
        autoLockTimeout: timeoutDraft,
        hotkey: hotkeyDraft,
      });
      setLockTimeout(timeoutDraft);
      setHotkey(hotkeyDraft);
      setSaved(true);
      setTimeout(() => setSaved(false), 1800);
    } finally {
      setSaving(false);
    }
  };

  const handleGenerateMcpToken = async () => {
    setGeneratingMcp(true);
    try {
      const token = await invoke<string>('vault_generate_mcp_token');
      setMcpToken(token);
      setMcpTokenVisible(true);
    } catch (e) {
      showToast('Failed to generate token');
    } finally {
      setGeneratingMcp(false);
    }
  };

  const handleWipe = async () => {
    setWiping(true);
    try {
      await storeWipe();
    } finally {
      setWiping(false);
      setWipeOpen(false);
    }
  };

  const handleBioEnable = async () => {
    if (!bioPw) return;
    setBioWorking(true);
    try {
      await invoke('biometric_enroll', { password: bioPw });
      setBioEnrolled(true);
      setBioPw('');
      showToast('Biometric unlock enabled');
    } catch (e: unknown) {
      showToast(e instanceof Error ? e.message : String(e));
    } finally {
      setBioWorking(false);
    }
  };

  const handleBioDisable = async () => {
    setBioWorking(true);
    try {
      await invoke('biometric_disable');
      setBioEnrolled(false);
      showToast('Biometric unlock disabled');
    } catch {
      showToast('Failed to disable biometric unlock');
    } finally {
      setBioWorking(false);
    }
  };

  const handleCheckUpdate = async () => {
    setCheckingUpdate(true);
    setUpdateStatus(null);
    setAvailableVersion(null);
    try {
      const version = await invoke<string | null>('check_for_update');
      if (version) {
        setAvailableVersion(version);
        setUpdateStatus('available');
      } else {
        setUpdateStatus('uptodate');
      }
    } catch (e) {
      showToast(String(e), 'error');
    } finally {
      setCheckingUpdate(false);
    }
  };

  const handleInstallUpdate = async () => {
    setInstallingUpdate(true);
    try {
      await invoke('install_update');
      showToast('Update installed — restart the app to apply');
    } catch (e) {
      showToast(String(e), 'error');
    } finally {
      setInstallingUpdate(false);
    }
  };

  return (
    <div className="flex-1 flex flex-col overflow-hidden animate-fade-in relative">
      {/* Header */}
      <div className="px-6 py-3 border-b border-bd flex items-center gap-3 shrink-0">
        <button
          onClick={() => go('vault')}
          className="flex items-center gap-1.5 text-[13px] font-medium font-ui text-tx3 bg-transparent border-none cursor-pointer hover:text-tx transition-colors"
        >
          <Icon name="back" size={13} />
          Back
        </button>
        <div className="flex-1 text-[13px] font-semibold text-center text-tx">Settings</div>
        <div className="w-[50px]" />
      </div>

      {/* Body */}
      <div className="flex-1 overflow-y-auto px-6 py-4 bg-surface">
        <Sec title="SECURITY" />
        <Row icon="key" label="Master Password">
          <button
            onClick={openChangePw}
            className="h-8 px-4 rounded-[3px] text-[12px] font-semibold tracking-[0.06em] font-ui cursor-pointer bg-transparent border border-bd2 text-tx2 hover:text-tx transition-colors"
          >
            CHANGE
          </button>
        </Row>
        {bioAvailable && (
          <Row icon="fingerprint" label="Biometric Unlock">
            {bioEnrolled ? (
              <div className="flex items-center gap-2">
                <span className="text-[12px] font-mono text-accent bg-accent-b border border-accent-d rounded-[3px] px-2 py-1">ENABLED</span>
                <button
                  onClick={handleBioDisable}
                  disabled={bioWorking}
                  className="h-8 px-4 rounded-[3px] text-[12px] font-semibold tracking-[0.06em] font-ui cursor-pointer bg-transparent border border-bd2 text-tx2 hover:text-tx transition-colors disabled:opacity-40"
                >
                  {bioWorking ? 'DISABLING…' : 'DISABLE'}
                </button>
              </div>
            ) : (
              <div className="flex items-center gap-2">
                <div className="relative">
                  <input
                    type={showBioPw ? 'text' : 'password'}
                    value={bioPw}
                    onChange={(e) => setBioPw(e.target.value)}
                    placeholder="Master password…"
                    autoComplete="off"
                    className={[
                      'bg-transparent border-0 border-b-2 border-bd2 text-tx font-mono text-[13px]',
                      'px-0 py-1.5 pr-7 outline-none focus:border-accent transition-colors duration-150 w-[148px]',
                    ].join(' ')}
                  />
                  <button
                    type="button"
                    onClick={() => setShowBioPw((v) => !v)}
                    className="absolute right-0 top-1/2 -translate-y-1/2 text-tx3 hover:text-tx transition-colors"
                  >
                    <Icon name={showBioPw ? 'eyeOff' : 'eye'} size={13} />
                  </button>
                </div>
                <button
                  onClick={handleBioEnable}
                  disabled={bioWorking || !bioPw}
                  className="h-8 px-4 rounded-[3px] text-[12px] font-semibold tracking-[0.06em] font-ui cursor-pointer bg-transparent border border-bd2 text-tx2 hover:text-tx transition-colors disabled:opacity-40 flex items-center gap-1.5"
                >
                  {bioWorking
                    ? <><div className="w-2.5 h-2.5 rounded-full border-2 border-transparent border-t-current animate-spin-fast" />ENABLING…</>
                    : 'ENABLE'}
                </button>
              </div>
            )}
          </Row>
        )}
        <Row icon="timer" label="Auto-lock Timeout">
          <select
            value={timeoutDraft}
            onChange={(e) => setTimeoutDraft(Number(e.target.value))}
            className="h-9 bg-raised border border-bd2 text-tx rounded-[3px] px-2 text-[13px] font-ui cursor-pointer outline-none focus:border-accent transition-colors"
          >
            {[{ v: 1, l: '1 min' }, { v: 5, l: '5 min' }, { v: 15, l: '15 min' }, { v: 30, l: '30 min' }, { v: 0, l: 'Never' }].map((o) => (
              <option key={o.v} value={o.v}>{o.l}</option>
            ))}
          </select>
        </Row>

        <Sec title="INTERFACE" />
        <Row icon="kbd" label="Global Hotkey">
          <button
            onClick={() => setCapturing(true)}
            onKeyDown={(e) => {
              if (!capturing) return;
              e.preventDefault();
              const m: string[] = [];
              if (e.ctrlKey)  m.push('Ctrl');
              if (e.altKey)   m.push('Alt');
              if (e.shiftKey) m.push('Shift');
              if (e.metaKey)  m.push('Meta');
              const k = e.key.length === 1 ? e.key.toUpperCase() : e.key;
              if (!['Control', 'Alt', 'Shift', 'Meta'].includes(e.key)) {
                setHotkeyDraft([...m, k].join('+'));
                setCapturing(false);
              }
            }}
            onBlur={() => setCapturing(false)}
            className={[
              'h-8 px-4 rounded-[3px] text-[12px] cursor-pointer font-mono tracking-[0.06em]',
              'border outline-none transition-all duration-150',
              capturing
                ? 'bg-accent-b border-accent-d text-accent animate-blink'
                : 'bg-raised border-bd2 text-tx hover:border-accent',
            ].join(' ')}
          >
            {capturing ? 'Press keys…' : hotkeyDraft}
          </button>
        </Row>
        <Row icon="tag" label="Manage Categories">
          <button
            onClick={() => go('categories')}
            className="flex items-center gap-1.5 h-8 px-4 bg-transparent border border-bd2 rounded-[3px] text-tx2 text-[12px] cursor-pointer font-ui font-semibold tracking-[0.06em] hover:text-tx transition-colors"
          >
            MANAGE →
          </button>
        </Row>

        <Sec title="PROJECTS" />
        <Row icon="terminal" label="Projects & Environments">
          <button
            onClick={() => go('projects')}
            className="flex items-center gap-1.5 h-8 px-4 bg-transparent border border-bd2 rounded-[3px] text-tx2 text-[12px] cursor-pointer font-ui font-semibold tracking-[0.06em] hover:text-tx transition-colors"
          >
            MANAGE →
          </button>
        </Row>

        <Sec title="DATA" />
        <Row icon="export" label="Import from Password Manager">
          <button
            onClick={() => setImportOpen(true)}
            className="h-8 px-4 bg-transparent border border-bd2 rounded-[3px] text-tx2 text-[12px] cursor-pointer font-ui font-semibold tracking-[0.06em] hover:text-tx transition-colors"
          >
            IMPORT
          </button>
        </Row>
        <Row icon="export" label="Backup & Restore">
          <button
            onClick={() => setBackupOpen(true)}
            className="h-8 px-4 bg-transparent border border-bd2 rounded-[3px] text-tx2 text-[12px] cursor-pointer font-ui font-semibold tracking-[0.06em] hover:text-tx transition-colors"
          >
            MANAGE
          </button>
        </Row>
        <Row icon="trash" label="Wipe All Data">
          <button
            onClick={() => setWipeOpen(true)}
            className="h-8 px-4 bg-danger-b border border-danger rounded-[3px] text-danger text-[12px] cursor-pointer font-ui font-semibold tracking-[0.06em] hover:opacity-80 transition-opacity"
          >
            WIPE
          </button>
        </Row>

        <Sec title="SHARE" />
        <Row icon="export" label="Receive / Import items">
          <button
            onClick={() => setShareOpen(true)}
            className="h-8 px-4 bg-transparent border border-bd2 rounded-[3px] text-tx2 text-[12px] cursor-pointer font-ui font-semibold tracking-[0.06em] hover:text-tx transition-colors"
          >
            OPEN
          </button>
        </Row>

        <Sec title="INTERNET SHARING" />
        <RelayConfigSection showToast={showToast} />

        <Sec title="INTEGRATIONS" />
        <Row icon="key" label="MCP Token">
          <button
            onClick={handleGenerateMcpToken}
            disabled={generatingMcp}
            className="h-8 px-4 bg-transparent border border-bd2 rounded-[3px] text-tx2 text-[12px] cursor-pointer font-ui font-semibold tracking-[0.06em] hover:text-tx transition-colors disabled:opacity-40 flex items-center gap-1.5"
          >
            {generatingMcp
              ? <><div className="w-2.5 h-2.5 rounded-full border-2 border-transparent border-t-current animate-spin-fast" />GEN…</>
              : mcpToken ? 'REGENERATE' : 'GENERATE'}
          </button>
        </Row>
        {mcpToken && (
          <div className="mt-2 mb-2 px-3 py-2.5 bg-raised border border-bd rounded-[3px] flex items-center gap-2">
            <code className="flex-1 text-[11px] font-mono text-tx2 truncate select-all">
              {mcpTokenVisible ? mcpToken : '••••••••••••••••••••••••••••••••'}
            </code>
            <button
              onClick={() => setMcpTokenVisible((v) => !v)}
              className="text-tx3 hover:text-tx transition-colors shrink-0"
              title={mcpTokenVisible ? 'Hide' : 'Show'}
              aria-label={mcpTokenVisible ? 'Hide token' : 'Show token'}
            >
              <Icon name={mcpTokenVisible ? 'eyeOff' : 'eye'} size={13} />
            </button>
            <button
              onClick={() => { navigator.clipboard.writeText(mcpToken); showToast('Token copied'); }}
              className="text-tx3 hover:text-tx transition-colors shrink-0"
              title="Copy"
              aria-label="Copy token"
            >
              <Icon name="copy" size={13} />
            </button>
          </div>
        )}

        <WslIntegrationSection isWindows={isWindows} />

        <Sec title="UPDATES" />
        <Row icon="export" label="Application Update">
          <button
            onClick={handleCheckUpdate}
            disabled={checkingUpdate || installingUpdate}
            className="h-8 px-4 bg-transparent border border-bd2 rounded-[3px] text-tx2 text-[12px] cursor-pointer font-ui font-semibold tracking-[0.06em] hover:text-tx transition-colors disabled:opacity-40 flex items-center gap-1.5"
          >
            {checkingUpdate
              ? <><div className="w-2.5 h-2.5 rounded-full border-2 border-transparent border-t-current animate-spin-fast" />CHECKING…</>
              : 'CHECK'}
          </button>
        </Row>
        {updateStatus === 'uptodate' && (
          <div className="mt-2 mb-2 px-3 py-2 rounded-[3px] border border-bd bg-raised text-[12px] font-mono text-tx3">
            Up to date
          </div>
        )}
        {updateStatus === 'available' && availableVersion && (
          <div className="mt-2 mb-2 flex items-center gap-3 px-3 py-2 rounded-[3px] border border-accent-d bg-accent-b">
            <span className="flex-1 text-[12px] font-mono text-accent">
              Version {availableVersion} available
            </span>
            <button
              onClick={handleInstallUpdate}
              disabled={installingUpdate}
              className="h-7 px-3 bg-accent border-none rounded-[3px] text-[#020504] text-[12px] font-bold tracking-[0.06em] font-ui cursor-pointer hover:opacity-90 disabled:opacity-40 flex items-center gap-1.5"
            >
              {installingUpdate
                ? <><div className="w-2.5 h-2.5 rounded-full border-2 border-transparent border-t-[#020504] animate-spin-fast" />INSTALLING…</>
                : 'INSTALL'}
            </button>
          </div>
        )}

        <div className="mt-8 mb-4 px-4 py-3 bg-raised border border-bd rounded-[3px]">
          <div className="text-[11px] text-tx3 font-mono leading-[1.9]">
            vault v2.0.0 · tauri 2.0 · rust 1.77<br />
            storage: ~/.vault/data.enc · argon2id m=65536 t=3
          </div>
        </div>
      </div>

      {/* Footer */}
      <div className="px-6 py-3 border-t border-bd shrink-0 bg-bg">
        <button
          onClick={handleSave}
          className={[
            'w-full h-10 rounded-[3px] text-[12px] font-bold tracking-[0.06em] cursor-pointer font-ui',
            'flex items-center justify-center gap-1.5 transition-all duration-200',
            saving
              ? 'bg-accent-d text-[#020504] border-none'
              : saved
              ? 'bg-accent-b border border-accent-d text-accent'
              : 'bg-accent border-none text-[#020504] hover:opacity-90',
          ].join(' ')}
        >
          {saving ? (
            <><div className="w-3 h-3 rounded-full border-2 border-transparent border-t-[#020504] animate-spin-fast" />SAVING…</>
          ) : saved ? (
            <><Icon name="check" size={13} color="oklch(0.70 0.17 162)" />SAVED</>
          ) : (
            'SAVE SETTINGS'
          )}
        </button>
      </div>

      {/* Wipe Confirmation Modal */}
      {wipeOpen && (
        <div className="absolute inset-0 bg-black/70 flex items-center justify-center z-20 p-6">
          <div className="w-full bg-surface border border-danger rounded-[4px] p-6">
            <div className="text-[12px] font-semibold text-danger font-mono tracking-[0.09em] mb-4">
              WIPE ALL DATA
            </div>
            <p className="text-[13px] text-tx mb-2 font-ui">
              This will permanently delete your vault database and all stored secrets.
            </p>
            <p className="text-[12px] text-tx3 font-mono mb-6">
              This action cannot be undone.
            </p>
            <div className="flex gap-3">
              <button
                onClick={() => setWipeOpen(false)}
                disabled={wiping}
                className="flex-1 h-10 rounded-[3px] text-[12px] font-bold tracking-[0.06em] font-ui cursor-pointer bg-transparent border border-bd2 text-tx2 hover:text-tx transition-colors disabled:opacity-40"
              >
                CANCEL
              </button>
              <button
                onClick={handleWipe}
                disabled={wiping}
                className="flex-1 h-10 rounded-[3px] text-[12px] font-bold tracking-[0.06em] font-ui cursor-pointer bg-danger border-none text-white hover:opacity-90 transition-opacity disabled:opacity-40 flex items-center justify-center gap-1.5"
              >
                {wiping
                  ? <><div className="w-3 h-3 rounded-full border-2 border-transparent border-t-white animate-spin-fast" />WIPING…</>
                  : 'CONFIRM WIPE'}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Change Master Password Modal */}
      {changePwOpen && (
        <div className="absolute inset-0 bg-black/70 flex items-center justify-center z-20 p-6">
          <div className="w-full bg-surface border border-bd rounded-[4px] p-6">
            <div className="text-[12px] font-semibold text-tx3 font-mono tracking-[0.09em] mb-5">
              CHANGE MASTER PASSWORD
            </div>

            <PwField
              label="CURRENT PASSWORD"
              value={currentPw}
              show={showCurrent}
              onChange={setCurrentPw}
              onToggle={() => setShowCurrent((v) => !v)}
            />
            <PwField
              label="NEW PASSWORD"
              value={newPw}
              show={showNew}
              onChange={setNewPw}
              onToggle={() => setShowNew((v) => !v)}
            />
            <PwField
              label="CONFIRM NEW PASSWORD"
              value={confirmPw}
              show={showConfirm}
              onChange={setConfirmPw}
              onToggle={() => setShowConfirm((v) => !v)}
            />

            {pwError && (
              <div className="mb-4 px-3 py-2.5 bg-danger-b border border-danger rounded-[3px] text-danger text-[12px] font-mono">
                {pwError}
              </div>
            )}

            <div className="flex gap-3 mt-2">
              <button
                onClick={closeChangePw}
                disabled={pwChanging}
                className="flex-1 h-10 rounded-[3px] text-[12px] font-bold tracking-[0.06em] font-ui cursor-pointer bg-transparent border border-bd2 text-tx2 hover:text-tx transition-colors disabled:opacity-40"
              >
                CANCEL
              </button>
              <button
                onClick={handleChangePassword}
                disabled={pwChanging || !currentPw || !newPw || !confirmPw}
                className="flex-1 h-10 rounded-[3px] text-[12px] font-bold tracking-[0.06em] font-ui cursor-pointer bg-accent border-none text-[#020504] hover:opacity-90 transition-opacity disabled:opacity-40 flex items-center justify-center gap-1.5"
              >
                {pwChanging
                  ? <><div className="w-3 h-3 rounded-full border-2 border-transparent border-t-[#020504] animate-spin-fast" />CHANGING…</>
                  : 'CONFIRM CHANGE'}
              </button>
            </div>
          </div>
        </div>
      )}

      {importOpen && <ImportModal onClose={() => setImportOpen(false)} />}
      {backupOpen && <BackupModal onClose={() => setBackupOpen(false)} />}
      {shareOpen  && <ReceiveModal onClose={() => setShareOpen(false)} />}
    </div>
  );
}
