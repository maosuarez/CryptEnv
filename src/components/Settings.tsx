import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { platform } from '@tauri-apps/plugin-os';
import { Icon } from './ui/Icon';
import { RelaySqlBlock, isRelaySchemaOutdated } from './ui/RelaySql';
import { ImportModal } from './ImportModal';
import { BackupModal } from './BackupModal';
import { ReceiveModal } from './ReceiveModal';
import { WslIntegrationSection } from './settings/WslIntegrationSection';
import { ApiStatusBanner } from './settings/ApiStatusBanner';
import { useVaultStore } from '../store';
import { copySecret } from '../lib/clipboard';
import { useSystemInfo } from '../hooks/useSystemInfo';
import { useThemeStore, type Theme } from '../store/themeStore';
import { LANGUAGES, LANGUAGE_NAMES, useTranslation, type Language } from '../i18n';
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

const defaultHotkey = (isMac: boolean) => (isMac ? 'Cmd+Alt+Z' : 'Ctrl+Alt+Z');

// Older builds stored `Meta`, which the shortcut parser rejects; show the
// platform-native name instead (the backend normalises it on save).
const displayHotkey = (hotkey: string, isMac: boolean) =>
  hotkey.split('+').map((k) => (k === 'Meta' ? (isMac ? 'Cmd' : 'Super') : k)).join('+');

// `KeyboardEvent.code` is layout- and Shift-independent (`KeyZ`, `Digit1`,
// `F5`, `ArrowUp`) and matches the names the Rust shortcut parser accepts.
const hotkeyKeyFromCode = (code: string) =>
  code.startsWith('Key') ? code.slice(3) : code.startsWith('Digit') ? code.slice(5) : code;

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

function AppearanceSection() {
  const { t, lang, setLang } = useTranslation();
  const theme    = useThemeStore((s) => s.theme);
  const setTheme = useThemeStore((s) => s.setTheme);
  const themes: Theme[] = ['dark', 'light'];

  return (
    <>
      <Sec title={t('appearance.section')} />
      <Row icon="eye" label={t('appearance.theme')}>
        <div className="flex border border-bd rounded-[3px] overflow-hidden">
          {themes.map((th) => (
            <button
              key={th}
              onClick={() => setTheme(th)}
              aria-pressed={theme === th}
              className={[
                'h-8 px-4 text-[12px] font-semibold tracking-[0.06em] font-ui border-none cursor-pointer transition-colors duration-150',
                theme === th
                  ? 'bg-accent text-[#020504]'
                  : 'bg-transparent text-tx3 hover:text-tx hover:bg-raised',
              ].join(' ')}
            >
              {t(th === 'dark' ? 'appearance.dark' : 'appearance.light')}
            </button>
          ))}
        </div>
      </Row>
      <Row icon="globe" label={t('appearance.language')}>
        <select
          value={lang}
          onChange={(e) => setLang(e.target.value as Language)}
          aria-label={t('appearance.language')}
          className="h-9 bg-raised border border-bd2 text-tx rounded-[3px] px-2 text-[13px] font-ui cursor-pointer outline-none focus:border-accent transition-colors"
        >
          {LANGUAGES.map((l) => (
            <option key={l} value={l}>{LANGUAGE_NAMES[l]}</option>
          ))}
        </select>
      </Row>
    </>
  );
}

function RelayConfigSection({ showToast }: { showToast: (msg: string, type?: 'success' | 'error') => void }) {
  const [relayMode, setRelayMode] = useState<'shared' | 'custom'>('shared');
  const [url,       setUrl]       = useState('');
  const [anonKey,   setAnonKey]   = useState('');
  const [showKey,   setShowKey]   = useState(false);
  const [sqlOpen,   setSqlOpen]   = useState(false);
  const [saving,    setSaving]    = useState(false);
  const [checking,  setChecking]  = useState(false);
  const [schemaStatus, setSchemaStatus] = useState<'unknown' | 'ok' | 'outdated'>('unknown');
  const { t } = useTranslation();
  // Persisted values: this save must not clobber (and re-register) them.
  const savedLockTimeout = useVaultStore((s) => s.lockTimeout);
  const savedHotkey      = useVaultStore((s) => s.hotkey);

  useEffect(() => {
    // Relay settings are persisted server-side; no need to load them here.
  }, []);

  const handleCheckSchema = async () => {
    setChecking(true);
    try {
      await invoke<number>('relay_schema_version');
      setSchemaStatus('ok');
    } catch (e) {
      const msg = String(e);
      if (isRelaySchemaOutdated(msg)) {
        setSchemaStatus('outdated');
        setSqlOpen(true);
      } else {
        setSchemaStatus('unknown');
        showToast(msg, 'error');
      }
    } finally {
      setChecking(false);
    }
  };

  const handleSaveRelay = async () => {
    if (!url.trim() || !anonKey.trim()) { showToast(t('settings.relay.required'), 'error'); return; }
    setSaving(true);
    try {
      await invoke('vault_save_settings', {
        autoLockTimeout: savedLockTimeout,
        hotkey: savedHotkey,
        relaySupabaseUrl: url.trim(),
        relaySupabaseAnonKey: anonKey.trim(),
      });
      showToast(t('settings.relay.saved'));
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
          {t('settings.relay.shared')}
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
          {t('settings.relay.custom')}
        </button>
      </div>

      {relayMode === 'shared' && (
        <div className="rounded-[3px] border border-bd bg-raised px-4 py-3 space-y-1.5">
          <div className="text-[13px] font-medium text-tx font-ui">{t('settings.relay.usingShared')}</div>
          <div className="text-[12px] font-mono text-tx3 leading-[1.9] space-y-0.5">
            <div>AES-256-GCM · Argon2id KDF</div>
            <div>{t('settings.relay.burnAfterRead')}</div>
            <div>{t('settings.relay.cannotRead')}</div>
          </div>
        </div>
      )}

      {relayMode === 'custom' && (
        <div className="space-y-4">
          <div className="flex items-start gap-2 px-3 py-2 rounded-[3px] border border-bd bg-raised">
            <span className="text-tx3 text-[13px] mt-0.5 shrink-0">ℹ</span>
            <p className="text-[12px] font-mono text-tx3 leading-[1.6]">
              {t('settings.relay.encryptedBeforeUpload')}
            </p>
          </div>

          <div>
            <div className="text-[11px] font-semibold text-tx3 font-mono tracking-[0.08em] mb-1.5">{t('settings.relay.url')}</div>
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
            <div className="text-[11px] font-semibold text-tx3 font-mono tracking-[0.08em] mb-1.5">{t('settings.relay.anonKey')}</div>
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
                ? <><div className="w-2.5 h-2.5 rounded-full border-2 border-transparent border-t-[#020504] animate-spin-fast" />{t('common.saving')}</>
                : t('settings.relay.save')}
            </button>
            <button
              onClick={() => setSqlOpen((v) => !v)}
              className="h-8 px-4 text-[12px] font-semibold tracking-[0.06em] font-ui text-tx3 border border-bd2 rounded-[3px] hover:text-tx transition-colors bg-transparent cursor-pointer"
            >
              {sqlOpen ? t('settings.relay.hideSql') : t('settings.relay.setupSql')}
            </button>
            <button
              onClick={handleCheckSchema}
              disabled={checking}
              className="h-8 px-4 text-[12px] font-semibold tracking-[0.06em] font-ui text-tx3 border border-bd2 rounded-[3px] hover:text-tx transition-colors bg-transparent cursor-pointer disabled:opacity-40"
            >
              {checking ? t('settings.relay.checking') : t('settings.relay.checkSchema')}
            </button>
          </div>

          {schemaStatus === 'ok' && (
            <div className="text-[11px] font-mono text-accent">{t('settings.relay.schemaOk')}</div>
          )}
          {schemaStatus === 'outdated' && (
            <div className="text-[11px] font-mono text-danger">{t('settings.relay.schemaOutdated')}</div>
          )}

          {sqlOpen && (
            <>
              <RelaySqlBlock onCopied={() => showToast(t('settings.relay.sqlCopied'))} />
              <div className="text-[11px] font-mono text-tx3">{t('settings.relay.migrateHint')}</div>
            </>
          )}
        </div>
      )}
    </div>
  );
}

export function Settings() {
  const go               = useVaultStore((s) => s.go);
  const goBack           = useVaultStore((s) => s.goBack);
  const sysInfo          = useSystemInfo();
  const { t }            = useTranslation();
  const showToast        = useVaultStore((s) => s.showToast);
  const storeWipe        = useVaultStore((s) => s.wipe);
  const storeLockTimeout = useVaultStore((s) => s.lockTimeout);
  const storeHotkey      = useVaultStore((s) => s.hotkey);
  const setLockTimeout   = useVaultStore((s) => s.setLockTimeout);
  const setHotkey        = useVaultStore((s) => s.setHotkey);

  const [timeoutDraft, setTimeoutDraft] = useState(storeLockTimeout);
  const [hotkeyDraft,  setHotkeyDraft]  = useState(storeHotkey);
  const [capturing,    setCapturing]    = useState(false);
  const [hotkeyError,  setHotkeyError]  = useState('');
  const [saving,       setSaving]       = useState(false);
  const [saved,        setSaved]        = useState(false);
  const [wipeOpen,     setWipeOpen]     = useState(false);
  const [wiping,       setWiping]       = useState(false);
  const [importOpen,   setImportOpen]   = useState(false);
  const [backupOpen,   setBackupOpen]   = useState(false);
  const [shareOpen,    setShareOpen]    = useState(false);
  const [isWindows]                     = useState(() => platform() === 'windows');
  const [isMac]                         = useState(() => platform() === 'macos');

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
  const [bioNotice,    setBioNotice]    = useState(false);
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
    if (newPw.length < 8) { setPwError(t('settings.toast.pwTooShort')); return; }
    if (newPw !== confirmPw) { setPwError(t('settings.toast.pwMismatch')); return; }
    setPwChanging(true);
    setPwError('');
    try {
      await invoke('vault_change_password', { currentPassword: currentPw, newPassword: newPw });
      closeChangePw();
      showToast(t('settings.toast.pwChanged'));
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
        invoke<boolean>('biometric_reenroll_notice').then(setBioNotice).catch(() => {});
      }
    }).catch(() => {});
  }, []);

  // While recording, suppress the global toggle so pressing the current
  // hotkey doesn't hide the window. The OS swallows that combination before
  // the webview sees it, so the backend echoes it via `hotkey_captured`.
  useEffect(() => {
    if (!capturing) return;
    invoke('vault_pause_hotkey', { paused: true }).catch(() => {});
    const unlisten = listen<string>('hotkey_captured', (e) => {
      setHotkeyDraft(e.payload);
      setHotkeyError('');
      setCapturing(false);
    });
    return () => {
      unlisten.then((f) => f()).catch(() => {});
      invoke('vault_pause_hotkey', { paused: false }).catch(() => {});
    };
  }, [capturing]);

  const handleHotkeyKeyDown = (e: React.KeyboardEvent<HTMLButtonElement>) => {
    if (!capturing) return;
    e.preventDefault();
    if (['Control', 'Alt', 'Shift', 'Meta'].includes(e.key)) return;
    const hasModifier = e.ctrlKey || e.altKey || e.metaKey;
    if (e.key === 'Escape' && !hasModifier) {
      setHotkeyError('');
      setCapturing(false);
      return;
    }
    if (!hasModifier) {
      setHotkeyError(t('settings.hotkey.needsModifier'));
      return;
    }
    const m: string[] = [];
    if (e.ctrlKey)  m.push('Ctrl');
    if (e.altKey)   m.push('Alt');
    if (e.shiftKey) m.push('Shift');
    if (e.metaKey)  m.push(isMac ? 'Cmd' : 'Super');
    setHotkeyDraft([...m, hotkeyKeyFromCode(e.code)].join('+'));
    setHotkeyError('');
    setCapturing(false);
  };

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
    } catch (e) {
      showToast(String(e), 'error');
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
      showToast(t('settings.toast.tokenFailed'));
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
      setBioNotice(false);
      setBioPw('');
      showToast(t('settings.toast.bioEnabled'));
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
      showToast(t('settings.toast.bioDisabled'));
    } catch {
      showToast(t('settings.toast.bioDisableFailed'));
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
      showToast(t('settings.toast.updateInstalled'));
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
          onClick={goBack}
          className="flex items-center gap-1.5 text-[13px] font-medium font-ui text-tx3 bg-transparent border-none cursor-pointer hover:text-tx transition-colors"
        >
          <Icon name="back" size={13} />
          {t('common.back')}
        </button>
        <div className="flex-1 text-[13px] font-semibold text-center text-tx">{t('settings.title')}</div>
        <div className="w-[50px]" />
      </div>

      {/* Body */}
      <div className="flex-1 overflow-y-auto px-6 py-4 bg-surface">
        <ApiStatusBanner />
        <Sec title={t('settings.sections.security')} />
        <Row icon="key" label={t('settings.rows.masterPassword')}>
          <button
            onClick={openChangePw}
            className="h-8 px-4 rounded-[3px] text-[12px] font-semibold tracking-[0.06em] font-ui cursor-pointer bg-transparent border border-bd2 text-tx2 hover:text-tx transition-colors"
          >
            {t('settings.btn.change')}
          </button>
        </Row>
        {bioAvailable && (
          <Row icon="fingerprint" label={t('settings.rows.biometric')}>
            {bioEnrolled ? (
              <div className="flex items-center gap-2">
                <span className="text-[12px] font-mono text-accent bg-accent-b border border-accent-d rounded-[3px] px-2 py-1">{t('settings.btn.enabled')}</span>
                <button
                  onClick={handleBioDisable}
                  disabled={bioWorking}
                  className="h-8 px-4 rounded-[3px] text-[12px] font-semibold tracking-[0.06em] font-ui cursor-pointer bg-transparent border border-bd2 text-tx2 hover:text-tx transition-colors disabled:opacity-40"
                >
                  {bioWorking ? t('settings.btn.disabling') : t('settings.btn.disable')}
                </button>
              </div>
            ) : (
              <div className="flex items-center gap-2">
                {bioNotice && (
                  <span className="text-[11px] text-tx2 max-w-[200px] leading-[1.5]">{t('settings.bioReenroll')}</span>
                )}
                <div className="relative">
                  <input
                    type={showBioPw ? 'text' : 'password'}
                    value={bioPw}
                    onChange={(e) => setBioPw(e.target.value)}
                    placeholder={t('settings.masterPwPlaceholder')}
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
                    ? <><div className="w-2.5 h-2.5 rounded-full border-2 border-transparent border-t-current animate-spin-fast" />{t('settings.btn.enabling')}</>
                    : t('settings.btn.enable')}
                </button>
              </div>
            )}
          </Row>
        )}
        <Row icon="timer" label={t('settings.rows.autoLock')}>
          <select
            value={timeoutDraft}
            onChange={(e) => setTimeoutDraft(Number(e.target.value))}
            className="h-9 bg-raised border border-bd2 text-tx rounded-[3px] px-2 text-[13px] font-ui cursor-pointer outline-none focus:border-accent transition-colors"
          >
            {[1, 5, 15, 30, 0].map((v) => ({ v, l: v === 0 ? t('settings.never') : t('settings.minutes', { n: v }) })).map((o) => (
              <option key={o.v} value={o.v}>{o.l}</option>
            ))}
          </select>
        </Row>

        <AppearanceSection />

        <Sec title={t('settings.sections.interface')} />
        <Row icon="kbd" label={t('settings.rows.hotkey')}>
          <div className="flex flex-col items-end gap-1">
            <div className="flex items-center gap-2">
              {hotkeyDraft !== defaultHotkey(isMac) && (
                <button
                  onClick={() => { setHotkeyDraft(defaultHotkey(isMac)); setHotkeyError(''); }}
                  className="h-8 px-3 bg-transparent border border-bd2 rounded-[3px] text-tx2 text-[12px] cursor-pointer font-ui font-semibold tracking-[0.06em] hover:text-tx transition-colors"
                >
                  {t('settings.hotkey.reset')}
                </button>
              )}
              <button
                onClick={() => { setHotkeyError(''); setCapturing(true); }}
                onKeyDown={handleHotkeyKeyDown}
                onBlur={() => { setCapturing(false); setHotkeyError(''); }}
                className={[
                  'h-8 px-4 rounded-[3px] text-[12px] cursor-pointer font-mono tracking-[0.06em]',
                  'border outline-none transition-all duration-150',
                  capturing
                    ? 'bg-accent-b border-accent-d text-accent animate-blink'
                    : 'bg-raised border-bd2 text-tx hover:border-accent',
                ].join(' ')}
              >
                {capturing ? t('settings.pressKeys') : displayHotkey(hotkeyDraft, isMac)}
              </button>
            </div>
            {hotkeyError && (
              <span role="alert" className="text-[11px] text-danger font-ui">{hotkeyError}</span>
            )}
          </div>
        </Row>
        <Row icon="tag" label={t('settings.rows.manageCategories')}>
          <button
            onClick={() => go('categories')}
            className="flex items-center gap-1.5 h-8 px-4 bg-transparent border border-bd2 rounded-[3px] text-tx2 text-[12px] cursor-pointer font-ui font-semibold tracking-[0.06em] hover:text-tx transition-colors"
          >
            {t('settings.btn.manageArrow')}
          </button>
        </Row>

        <Sec title={t('settings.sections.projects')} />
        <Row icon="terminal" label={t('settings.rows.projects')}>
          <button
            onClick={() => go('projects')}
            className="flex items-center gap-1.5 h-8 px-4 bg-transparent border border-bd2 rounded-[3px] text-tx2 text-[12px] cursor-pointer font-ui font-semibold tracking-[0.06em] hover:text-tx transition-colors"
          >
            {t('settings.btn.manageArrow')}
          </button>
        </Row>

        <Sec title={t('settings.sections.data')} />
        <Row icon="export" label={t('settings.rows.importPm')}>
          <button
            onClick={() => setImportOpen(true)}
            className="h-8 px-4 bg-transparent border border-bd2 rounded-[3px] text-tx2 text-[12px] cursor-pointer font-ui font-semibold tracking-[0.06em] hover:text-tx transition-colors"
          >
            {t('settings.btn.import')}
          </button>
        </Row>
        <Row icon="export" label={t('settings.rows.backup')}>
          <button
            onClick={() => setBackupOpen(true)}
            className="h-8 px-4 bg-transparent border border-bd2 rounded-[3px] text-tx2 text-[12px] cursor-pointer font-ui font-semibold tracking-[0.06em] hover:text-tx transition-colors"
          >
            {t('settings.btn.manage')}
          </button>
        </Row>
        <Row icon="trash" label={t('settings.rows.wipe')}>
          <button
            onClick={() => setWipeOpen(true)}
            className="h-8 px-4 bg-danger-b border border-danger rounded-[3px] text-danger text-[12px] cursor-pointer font-ui font-semibold tracking-[0.06em] hover:opacity-80 transition-opacity"
          >
            {t('settings.btn.wipe')}
          </button>
        </Row>

        <Sec title={t('settings.sections.share')} />
        <Row icon="export" label={t('settings.rows.receive')}>
          <button
            onClick={() => setShareOpen(true)}
            className="h-8 px-4 bg-transparent border border-bd2 rounded-[3px] text-tx2 text-[12px] cursor-pointer font-ui font-semibold tracking-[0.06em] hover:text-tx transition-colors"
          >
            {t('settings.btn.open')}
          </button>
        </Row>

        <Sec title={t('settings.sections.internetSharing')} />
        <RelayConfigSection showToast={showToast} />

        <Sec title={t('settings.sections.integrations')} />
        <Row icon="key" label={t('settings.rows.mcpToken')}>
          <button
            onClick={handleGenerateMcpToken}
            disabled={generatingMcp}
            className="h-8 px-4 bg-transparent border border-bd2 rounded-[3px] text-tx2 text-[12px] cursor-pointer font-ui font-semibold tracking-[0.06em] hover:text-tx transition-colors disabled:opacity-40 flex items-center gap-1.5"
          >
            {generatingMcp
              ? <><div className="w-2.5 h-2.5 rounded-full border-2 border-transparent border-t-current animate-spin-fast" />{t('settings.btn.generatingShort')}</>
              : mcpToken ? t('settings.btn.regenerate') : t('settings.btn.generate')}
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
              title={mcpTokenVisible ? t('common.hide') : t('common.show')}
              aria-label={mcpTokenVisible ? t('settings.hideToken') : t('settings.showToken')}
            >
              <Icon name={mcpTokenVisible ? 'eyeOff' : 'eye'} size={13} />
            </button>
            <button
              onClick={() => { copySecret(mcpToken, t('settings.toast.tokenCopied')).catch(() => {}); }}
              className="text-tx3 hover:text-tx transition-colors shrink-0"
              title={t('common.copy')}
              aria-label={t('settings.copyToken')}
            >
              <Icon name="copy" size={13} />
            </button>
          </div>
        )}

        <WslIntegrationSection isWindows={isWindows} />

        <Sec title={t('settings.sections.updates')} />
        <Row icon="export" label={t('settings.rows.appUpdate')}>
          <button
            onClick={handleCheckUpdate}
            disabled={checkingUpdate || installingUpdate}
            className="h-8 px-4 bg-transparent border border-bd2 rounded-[3px] text-tx2 text-[12px] cursor-pointer font-ui font-semibold tracking-[0.06em] hover:text-tx transition-colors disabled:opacity-40 flex items-center gap-1.5"
          >
            {checkingUpdate
              ? <><div className="w-2.5 h-2.5 rounded-full border-2 border-transparent border-t-current animate-spin-fast" />{t('settings.btn.checking')}</>
              : t('settings.btn.check')}
          </button>
        </Row>
        {updateStatus === 'uptodate' && (
          <div className="mt-2 mb-2 px-3 py-2 rounded-[3px] border border-bd bg-raised text-[12px] font-mono text-tx3">
            {t('settings.upToDate')}
          </div>
        )}
        {updateStatus === 'available' && availableVersion && (
          <div className="mt-2 mb-2 flex items-center gap-3 px-3 py-2 rounded-[3px] border border-accent-d bg-accent-b">
            <span className="flex-1 text-[12px] font-mono text-accent">
              {t('settings.versionAvailable', { version: availableVersion })}
            </span>
            <button
              onClick={handleInstallUpdate}
              disabled={installingUpdate}
              className="h-7 px-3 bg-accent border-none rounded-[3px] text-[#020504] text-[12px] font-bold tracking-[0.06em] font-ui cursor-pointer hover:opacity-90 disabled:opacity-40 flex items-center gap-1.5"
            >
              {installingUpdate
                ? <><div className="w-2.5 h-2.5 rounded-full border-2 border-transparent border-t-[#020504] animate-spin-fast" />{t('settings.btn.installing')}</>
                : t('settings.btn.install')}
            </button>
          </div>
        )}

        <div className="mt-8 mb-4 px-4 py-3 bg-raised border border-bd rounded-[3px]">
          <div className="text-[11px] text-tx3 font-mono leading-[1.9]">
            CryptEnv{sysInfo.version ? ` v${sysInfo.version}` : ''}{sysInfo.os ? ` · ${sysInfo.os}` : ''}<br />
            AES-256-GCM · Argon2id m=65536 t=3 p=4<br />
            <span className="break-all">{t('appearance.storage', { path: sysInfo.dbPath ?? '…' })}</span>
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
            <><div className="w-3 h-3 rounded-full border-2 border-transparent border-t-[#020504] animate-spin-fast" />{t('common.saving')}</>
          ) : saved ? (
            <><Icon name="check" size={13} color="oklch(0.70 0.17 162)" />{t('common.saved')}</>
          ) : (
            t('settings.btn.saveSettings')
          )}
        </button>
      </div>

      {/* Wipe Confirmation Modal */}
      {wipeOpen && (
        <div className="absolute inset-0 bg-black/70 flex items-center justify-center z-20 p-6">
          <div className="w-full bg-surface border border-danger rounded-[4px] p-6">
            <div className="text-[12px] font-semibold text-danger font-mono tracking-[0.09em] mb-4">
              {t('settings.wipeDialog.title')}
            </div>
            <p className="text-[13px] text-tx mb-2 font-ui">
              {t('settings.wipeDialog.body')}
            </p>
            <p className="text-[12px] text-tx3 font-mono mb-6">
              {t('settings.wipeDialog.irreversible')}
            </p>
            <div className="flex gap-3">
              <button
                onClick={() => setWipeOpen(false)}
                disabled={wiping}
                className="flex-1 h-10 rounded-[3px] text-[12px] font-bold tracking-[0.06em] font-ui cursor-pointer bg-transparent border border-bd2 text-tx2 hover:text-tx transition-colors disabled:opacity-40"
              >
                {t('common.cancel')}
              </button>
              <button
                onClick={handleWipe}
                disabled={wiping}
                className="flex-1 h-10 rounded-[3px] text-[12px] font-bold tracking-[0.06em] font-ui cursor-pointer bg-danger border-none text-white hover:opacity-90 transition-opacity disabled:opacity-40 flex items-center justify-center gap-1.5"
              >
                {wiping
                  ? <><div className="w-3 h-3 rounded-full border-2 border-transparent border-t-white animate-spin-fast" />{t('settings.btn.wiping')}</>
                  : t('settings.btn.confirmWipe')}
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
              {t('settings.pwDialog.title')}
            </div>

            <PwField
              label={t('settings.pwDialog.current')}
              value={currentPw}
              show={showCurrent}
              onChange={setCurrentPw}
              onToggle={() => setShowCurrent((v) => !v)}
            />
            <PwField
              label={t('settings.pwDialog.new')}
              value={newPw}
              show={showNew}
              onChange={setNewPw}
              onToggle={() => setShowNew((v) => !v)}
            />
            <PwField
              label={t('settings.pwDialog.confirm')}
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
                {t('common.cancel')}
              </button>
              <button
                onClick={handleChangePassword}
                disabled={pwChanging || !currentPw || !newPw || !confirmPw}
                className="flex-1 h-10 rounded-[3px] text-[12px] font-bold tracking-[0.06em] font-ui cursor-pointer bg-accent border-none text-[#020504] hover:opacity-90 transition-opacity disabled:opacity-40 flex items-center justify-center gap-1.5"
              >
                {pwChanging
                  ? <><div className="w-3 h-3 rounded-full border-2 border-transparent border-t-[#020504] animate-spin-fast" />{t('settings.btn.changing')}</>
                  : t('settings.btn.confirmChange')}
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
