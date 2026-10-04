import { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { writeText } from '@tauri-apps/plugin-clipboard-manager';
import { Icon } from './ui/Icon';
import { RelayCodeDisplay } from './ui/RelayCodeDisplay';
import { useTranslation } from '../i18n';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type Method = 'lan' | 'file' | 'internet';
type LanRole = 'send' | 'receive';

type SessionState =
  | 'listening'
  | 'connecting'
  | 'awaiting_fingerprint'
  | 'active'
  | 'done'
  | 'failed'
  | 'cancelled';

interface PollResult {
  state: SessionState;
  fingerprint?: string;
  receivedNames?: string[];
  error?: string;
  note?: string;
}

type Step =
  | 'method'
  | 'lan-send'
  | 'lan-receive'
  | 'fingerprint'
  | 'done-send'
  | 'done-receive'
  | 'done-file-export'
  | 'done-file-import'
  | 'failed'
  | 'file-export'
  | 'file-import'
  | 'internet-send'
  | 'internet-receive'
  | 'internet-done-send'
  | 'internet-done-receive';

export interface ShareModalProps {
  selectedIds: number[];
  onClose: () => void;
  onImportDone: () => void;
  onSendDone?: () => void;
}

// ---------------------------------------------------------------------------
// Small primitives
// ---------------------------------------------------------------------------

function SectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <div className="text-[10px] font-mono text-tx3 tracking-[0.12em] uppercase mb-3">
      {children}
    </div>
  );
}

function Divider() {
  return <div className="border-t border-bd my-4" />;
}

function Spinner() {
  return (
    <div className="w-4 h-4 rounded-full border-2 border-bd2 border-t-accent animate-spin-fast shrink-0" />
  );
}

function BtnPrimary({
  children,
  onClick,
  disabled,
  className = '',
}: {
  children: React.ReactNode;
  onClick?: () => void;
  disabled?: boolean;
  className?: string;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className={[
        'flex items-center justify-center gap-2 rounded-[3px] px-4 h-9',
        'bg-accent text-[#020504] text-[11px] font-bold tracking-wider font-ui',
        'cursor-pointer transition-opacity shrink-0',
        'disabled:opacity-30 disabled:cursor-not-allowed hover:opacity-90',
        className,
      ].join(' ')}
    >
      {children}
    </button>
  );
}

function BtnSecondary({
  children,
  onClick,
  disabled,
  className = '',
}: {
  children: React.ReactNode;
  onClick?: () => void;
  disabled?: boolean;
  className?: string;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className={[
        'flex items-center justify-center gap-2 rounded-[3px] px-4 h-9',
        'border border-bd2 text-tx2 text-[11px] font-medium tracking-wider font-ui bg-transparent',
        'cursor-pointer transition-all duration-150 shrink-0',
        'disabled:opacity-30 disabled:cursor-not-allowed hover:border-tx3 hover:text-tx',
        className,
      ].join(' ')}
    >
      {children}
    </button>
  );
}

function BtnDanger({
  children,
  onClick,
  disabled,
}: {
  children: React.ReactNode;
  onClick?: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className={[
        'flex items-center justify-center gap-2 rounded-[3px] px-4 h-9',
        'bg-danger-b border border-danger text-danger',
        'text-[11px] font-bold tracking-wider font-ui',
        'cursor-pointer transition-opacity shrink-0',
        'disabled:opacity-30 disabled:cursor-not-allowed hover:opacity-80',
      ].join(' ')}
    >
      {children}
    </button>
  );
}

function InlineError({ msg }: { msg: string }) {
  return (
    <div className="text-[12px] text-danger font-mono bg-danger-b border border-danger rounded-[3px] px-3 py-2">
      {msg}
    </div>
  );
}

// Breadcrumb path indicator
function Breadcrumb({ path }: { path: string }) {
  return (
    <div className="text-[10px] font-mono text-tx3 tracking-[0.08em] mb-4">{path}</div>
  );
}

// 6-digit pairing code display
function PairingCodeDisplay({ code }: { code: string }) {
  const digits = code.replace(/\s/g, '').split('');
  return (
    <div className="flex items-center gap-2 justify-center my-4">
      {digits.map((d, i) => (
        <div
          key={i}
          className="w-9 h-11 bg-raised border border-bd2 rounded-[3px] flex items-center justify-center text-[24px] font-mono text-accent select-all"
        >
          {d}
        </div>
      ))}
    </div>
  );
}

// 6 individual input boxes for pairing code entry
function PairingCodeInput({
  value,
  onChange,
  onComplete,
}: {
  value: string;
  onChange: (v: string) => void;
  onComplete: () => void;
}) {
  const refs = useRef<(HTMLInputElement | null)[]>([]);
  const digits = Array.from({ length: 6 }, (_, i) => value[i] ?? '');

  const handleKey = (i: number, e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Backspace') {
      e.preventDefault();
      const next = value.slice(0, i) + value.slice(i + 1);
      onChange(next);
      if (i > 0) refs.current[i - 1]?.focus();
    }
  };

  const handleInput = (i: number, e: React.ChangeEvent<HTMLInputElement>) => {
    const raw = e.target.value.replace(/\D/g, '');
    if (!raw) return;
    const ch = raw[raw.length - 1];
    const next = (value.slice(0, i) + ch + value.slice(i + 1)).slice(0, 6);
    onChange(next);
    if (i < 5) {
      refs.current[i + 1]?.focus();
    } else if (next.length === 6) {
      onComplete();
    }
  };

  const handlePaste = (e: React.ClipboardEvent) => {
    const pasted = e.clipboardData.getData('text').replace(/\D/g, '').slice(0, 6);
    if (pasted.length > 0) {
      onChange(pasted);
      const focusIdx = Math.min(pasted.length, 5);
      refs.current[focusIdx]?.focus();
      if (pasted.length === 6) onComplete();
    }
    e.preventDefault();
  };

  return (
    <div className="flex items-center gap-2 justify-center my-4">
      {digits.map((d, i) => (
        <input
          key={i}
          ref={(el) => { refs.current[i] = el; }}
          type="text"
          inputMode="numeric"
          maxLength={1}
          value={d === ' ' ? '' : d}
          onChange={(e) => handleInput(i, e)}
          onKeyDown={(e) => handleKey(i, e)}
          onPaste={handlePaste}
          className={[
            'w-10 h-12 bg-surface border-2 rounded-[3px] text-center text-[24px] font-mono text-accent',
            'outline-none transition-colors duration-100',
            d !== ' ' && d !== '' ? 'border-accent-d bg-accent-b' : 'border-bd2',
            'focus:border-accent focus:bg-accent-b',
          ].join(' ')}
        />
      ))}
    </div>
  );
}

// Fingerprint display
function FingerprintDisplay({ fp }: { fp: string }) {
  return (
    <div className="bg-raised border border-bd2 rounded-[3px] px-4 py-3 text-center my-3">
      <span className="text-[16px] font-mono text-accent tracking-[0.08em] select-all">
        {fp}
      </span>
    </div>
  );
}

// Countdown timer component
function Countdown({ seconds }: { seconds: number }) {
  const [remaining, setRemaining] = useState(seconds);

  useEffect(() => {
    setRemaining(seconds);
    const id = setInterval(() => {
      setRemaining((r) => {
        if (r <= 1) { clearInterval(id); return 0; }
        return r - 1;
      });
    }, 1000);
    return () => clearInterval(id);
  }, [seconds]);

  const m = Math.floor(remaining / 60);
  const s = remaining % 60;
  const display = `${m}:${s.toString().padStart(2, '0')}`;
  const urgent = remaining < 60;

  return (
    <span className={['font-mono text-[12px]', urgent ? 'text-danger' : 'text-tx3'].join(' ')}>
      {display}
    </span>
  );
}

// ---------------------------------------------------------------------------
// Main modal
// ---------------------------------------------------------------------------

export function ShareModal({ selectedIds, onClose, onImportDone, onSendDone }: ShareModalProps) {
  const { t } = useTranslation();
  const [step, setStep] = useState<Step>('method');
  const [method, setMethod] = useState<Method | null>(null);
  const [lanRole, setLanRole] = useState<LanRole | null>(null);

  // LAN send state
  const [pairingCode, setPairingCode] = useState('');
  // LAN receive state
  const [inputCode, setInputCode] = useState('');
  const [receiveStatus, setReceiveStatus] = useState<'idle' | 'loading' | 'connecting'>('idle');

  // Fingerprint state
  const [fingerprint, setFingerprint] = useState('');
  const [fpConfirming, setFpConfirming] = useState(false);

  // Done state
  const [receivedNames, setReceivedNames] = useState<string[]>([]);
  const [sentCount, setSentCount] = useState(0);

  // File export state
  const [exportPassphrase, setExportPassphrase] = useState('');
  const [exportLoading, setExportLoading] = useState(false);
  const [copiedPass, setCopiedPass] = useState(false);

  // File import state
  const [importPassphrase, setImportPassphrase] = useState('');
  const [importFileReady, setImportFileReady] = useState(false);
  const [importLoading, setImportLoading] = useState(false);
  const [importedNames, setImportedNames] = useState<string[]>([]);

  // Internet relay state
  const [relayCode,        setRelayCode]        = useState('');
  const [relayPassphrase,  setRelayPassphrase]  = useState('');
  const [relayLoading,     setRelayLoading]     = useState(false);
  const [relayRxCode,      setRelayRxCode]      = useState('');
  const [relayRxPass,      setRelayRxPass]      = useState('');
  const [relayRxLoading,   setRelayRxLoading]   = useState(false);
  const [relayRxNames,     setRelayRxNames]     = useState<string[]>([]);

  // Error state
  const [error, setError] = useState('');
  const [failedError, setFailedError] = useState('');

  // Informational note from the backend (e.g. Windows Firewall reminder)
  const [sessionNote, setSessionNote] = useState('');

  // Polling
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const stopPolling = useCallback(() => {
    if (pollRef.current !== null) {
      clearInterval(pollRef.current);
      pollRef.current = null;
    }
  }, []);

  useEffect(() => {
    return () => {
      stopPolling();
      // Closing the modal ends any LAN session (frees the port, drops the key copy).
      invoke('share_cancel').catch(() => { /* best effort */ });
    };
  }, [stopPolling]);

  const startPolling = useCallback(() => {
    stopPolling();
    pollRef.current = setInterval(async () => {
      try {
        const result = await invoke<PollResult>('share_poll_status');
        // Surface backend notes (e.g. Windows Firewall reminder) immediately
        if (result.note) setSessionNote(result.note);
        if (result.state === 'awaiting_fingerprint' && result.fingerprint) {
          setFingerprint(result.fingerprint);
          stopPolling();
          setStep('fingerprint');
        } else if (result.state === 'done') {
          stopPolling();
          if (result.receivedNames && result.receivedNames.length > 0) {
            setReceivedNames(result.receivedNames);
            setStep('done-receive');
          } else {
            setStep('done-send');
          }
        } else if (result.state === 'failed') {
          stopPolling();
          setFailedError(result.error ?? t('share.transferFailed'));
          setStep('failed');
        } else if (result.state === 'cancelled') {
          stopPolling();
          onClose();
        } else if (result.state === 'active') {
          // actively transferring — keep polling
        }
      } catch (e) {
        stopPolling();
        setFailedError(String(e));
        setStep('failed');
      }
    }, 800);
  }, [stopPolling, onClose, t]);

  // ------------------------------------------------------------------
  // Action handlers
  // ------------------------------------------------------------------

  const handleCancel = async () => {
    stopPolling();
    try { await invoke('share_cancel'); } catch { /* best effort */ }
    onClose();
  };


  const handleLanSend = async () => {
    setLanRole('send');
    setSentCount(selectedIds.length);
    setError('');
    try {
      const res = await invoke<{ pairingCode: string }>('share_start_send', { itemIds: selectedIds });
      setPairingCode(res.pairingCode);
      setStep('lan-send');
      startPolling();
    } catch (e) {
      setError(String(e));
    }
  };


  const handleReceiveSubmit = async () => {
    if (inputCode.length < 6) return;
    setReceiveStatus('loading');
    setError('');
    try {
      await invoke<{ fingerprint: string }>('share_start_receive', { pairingCode: inputCode });
      setReceiveStatus('connecting');
      startPolling();
    } catch (e) {
      setError(String(e));
      setReceiveStatus('idle');
    }
  };

  const handleFingerprintConfirm = async (confirmed: boolean) => {
    setFpConfirming(true);
    try {
      await invoke('share_confirm_fingerprint', { confirmed });
      if (!confirmed) {
        onClose();
        return;
      }
      // If confirmed, start polling again for done/failed
      startPolling();
    } catch (e) {
      setError(String(e));
    } finally {
      setFpConfirming(false);
    }
  };

  const handleFileExport = async () => {
    setExportLoading(true);
    setError('');
    try {
      const res = await invoke<{ passphrase: string }>('share_export_file', { itemIds: selectedIds });
      setExportPassphrase(res.passphrase);
      setStep('done-file-export');
    } catch (e) {
      setError(String(e));
    } finally {
      setExportLoading(false);
    }
  };

  const handleFileImportOpen = async () => {
    setImportLoading(true);
    setError('');
    try {
      // The Rust side opens a native file dialog first, then waits for passphrase
      // We just trigger the flow — actual import happens on submit
      setImportFileReady(true);
    } finally {
      setImportLoading(false);
    }
  };

  const handleFileImportSubmit = async () => {
    if (!importPassphrase) return;
    setImportLoading(true);
    setError('');
    try {
      const res = await invoke<{ names: string[] }>('share_import_file', { passphrase: importPassphrase });
      setImportedNames(res.names);
      setStep('done-file-import');
    } catch (e) {
      setError(String(e));
    } finally {
      setImportLoading(false);
    }
  };

  const handleCopyPassphrase = async () => {
    try {
      await writeText(exportPassphrase);
      setCopiedPass(true);
      setTimeout(() => setCopiedPass(false), 2000);
    } catch { /* ignore */ }
  };

  const handleImportDoneClose = () => {
    onImportDone();
    onClose();
  };

  // ------------------------------------------------------------------
  // Render helpers
  // ------------------------------------------------------------------

  // ------------------------------------------------------------------
  // Step renderers
  // ------------------------------------------------------------------

  function renderMethod() {
    return (
      <>
        <div className="flex gap-3 mb-4">
          {/* LAN Card */}
          <button
            onClick={() => { setMethod('lan'); setError(''); }}
            className={[
              'flex-1 border rounded-[3px] p-4 text-left transition-all duration-150 cursor-pointer',
              method === 'lan'
                ? 'bg-accent-b border-accent-d'
                : 'bg-raised border-bd hover:border-bd2',
            ].join(' ')}
          >
            <div className="flex items-center gap-2 mb-2">
              <Icon name="globe" size={15} color={method === 'lan' ? 'oklch(0.70 0.17 162)' : 'var(--color-tx3)'} />
              <span className={['text-[12px] font-bold tracking-wider font-ui', method === 'lan' ? 'text-accent' : 'text-tx'].join(' ')}>
                LAN
              </span>
            </div>
            <div className="text-[11px] text-tx2 leading-[1.5]">{t('share.method.lanDesc')}</div>
            <div className="text-[10px] text-tx3 mt-1 font-mono">{t('share.method.lanMeta')}</div>
          </button>

          {/* File Card */}
          <button
            onClick={() => { setMethod('file'); setError(''); }}
            className={[
              'flex-1 border rounded-[3px] p-4 text-left transition-all duration-150 cursor-pointer',
              method === 'file'
                ? 'bg-accent-b border-accent-d'
                : 'bg-raised border-bd hover:border-bd2',
            ].join(' ')}
          >
            <div className="flex items-center gap-2 mb-2">
              <Icon name="export" size={15} color={method === 'file' ? 'oklch(0.70 0.17 162)' : 'var(--color-tx3)'} />
              <span className={['text-[12px] font-bold tracking-wider font-ui', method === 'file' ? 'text-accent' : 'text-tx'].join(' ')}>
                {t('share.method.file')}
              </span>
            </div>
            <div className="text-[11px] text-tx2 leading-[1.5]">{t('share.method.fileDesc')}</div>
            <div className="text-[10px] text-tx3 mt-1 font-mono">{t('share.method.fileMeta')}</div>
          </button>
        </div>

        {/* Internet Card (full-width row) */}
        <button
          onClick={() => { setMethod('internet'); setError(''); }}
          className={[
            'w-full border rounded-[3px] p-3 text-left transition-all duration-150 cursor-pointer mb-4',
            method === 'internet'
              ? 'bg-accent-b border-accent-d'
              : 'bg-raised border-bd hover:border-bd2',
          ].join(' ')}
        >
          <div className="flex items-center gap-3">
            <Icon name="shield" size={15} color={method === 'internet' ? 'oklch(0.70 0.17 162)' : 'var(--color-tx3)'} />
            <div className="flex-1">
              <span className={['text-[12px] font-bold tracking-wider font-ui', method === 'internet' ? 'text-accent' : 'text-tx'].join(' ')}>
                {t('share.method.internet')}
              </span>
              <span className="text-[10px] text-tx3 font-mono ml-3">{t('share.method.internetMeta')}</span>
            </div>
          </div>
        </button>

        {/* Item badge */}
        <div className="flex items-center gap-2 mb-4">
          <span className="w-1.5 h-1.5 rounded-full bg-accent shrink-0" />
          <span className="text-[11px] text-tx2 font-mono">
            {t('share.sharing')} <span className="text-accent font-bold">{selectedIds.length}</span> {t(selectedIds.length === 1 ? 'share.itemWord_one' : 'share.itemWord_other')}
          </span>
        </div>

        {/* LAN sub-actions */}
        {method === 'lan' && (
          <BtnPrimary onClick={handleLanSend} className="w-full">
            <Icon name="export" size={12} color="#020504" />
            {t('share.sendViaLan')}
          </BtnPrimary>
        )}

        {/* FILE sub-actions */}
        {method === 'file' && (
          <BtnPrimary onClick={() => setStep('file-export')} className="w-full">
            <Icon name="export" size={12} color="#020504" />
            {t('share.exportFile')}
          </BtnPrimary>
        )}

        {/* INTERNET sub-actions */}
        {method === 'internet' && (
          <div className="flex gap-2">
            <BtnPrimary onClick={() => setStep('internet-send')} className="flex-1" disabled={selectedIds.length === 0}>
              <Icon name="export" size={12} color="#020504" />
              {t('share.send')}
            </BtnPrimary>
            <BtnSecondary onClick={() => setStep('internet-receive')} className="flex-1">
              <Icon name="back" size={12} />
              {t('share.receive')}
            </BtnSecondary>
          </div>
        )}

        {error && <div className="mt-3"><InlineError msg={error} /></div>}
      </>
    );
  }

  function renderLanSend() {
    return (
      <>
        <Breadcrumb path={t('share.crumb.lanSend')} />
        <SectionLabel>{t('share.waitingForPeer')}</SectionLabel>
        <div className="flex items-center gap-2 text-[11px] text-tx3 font-mono mb-1">
          <Spinner />
          <span>{t('share.listening')}</span>
        </div>
        <PairingCodeDisplay code={pairingCode} />
        <div className="text-center text-[11px] text-tx3 font-mono mb-1">
          {t('share.shareCode')}
        </div>
        <div className="flex items-center justify-center gap-1.5 mb-4">
          <span className="text-[11px] text-tx3 font-mono">{t('share.connectionTimeout')}</span>
          <Countdown seconds={300} />
        </div>
        {sessionNote && (
          <div className="mb-3 rounded-[3px] border border-warn-d bg-warn-b px-3 py-2 text-[11px] text-warn font-mono leading-relaxed">
            {sessionNote}
          </div>
        )}
        <Divider />
        <div className="flex justify-end">
          <BtnSecondary onClick={handleCancel}>{t('common.cancel')}</BtnSecondary>
        </div>
      </>
    );
  }

  function renderLanReceive() {
    const isConnecting = receiveStatus === 'connecting';
    const isLoading    = receiveStatus === 'loading';

    return (
      <>
        <Breadcrumb path={t('share.crumb.lanReceive')} />
        <SectionLabel>{t('share.enterPairingCode')}</SectionLabel>
        <PairingCodeInput
          value={inputCode}
          onChange={setInputCode}
          onComplete={() => { if (!isConnecting && !isLoading) handleReceiveSubmit(); }}
        />
        {isConnecting && (
          <div className="flex items-center justify-center gap-2 text-[11px] text-tx3 font-mono mb-2">
            <Spinner />
            <span>{t('share.connecting')}</span>
          </div>
        )}
        {error && <div className="mb-3"><InlineError msg={error} /></div>}
        <Divider />
        <div className="flex justify-between">
          <BtnSecondary onClick={() => { setStep('method'); setError(''); }}>{t('share.back')}</BtnSecondary>
          <BtnPrimary
            onClick={handleReceiveSubmit}
            disabled={inputCode.length < 6 || isLoading || isConnecting}
          >
            {isLoading ? <Spinner /> : null}
            {t('share.connect')}
          </BtnPrimary>
        </div>
      </>
    );
  }

  function renderFingerprint() {
    return (
      <>
        <Breadcrumb path={t(lanRole === 'send' ? 'share.crumb.lanSendVerify' : 'share.crumb.lanReceiveVerify')} />
        <SectionLabel>{t('share.verifyFingerprint')}</SectionLabel>
        <p className="text-[12px] text-tx2 mb-3 leading-[1.6]">
          {t('share.verifyPrompt')}
        </p>
        <FingerprintDisplay fp={fingerprint} />
        <p className="text-[11px] text-tx3 font-mono mt-2 mb-4 leading-[1.5]">
          {t('share.verifyHint')}
        </p>
        {error && <div className="mb-3"><InlineError msg={error} /></div>}
        <Divider />
        <div className="flex gap-2 justify-between">
          <BtnDanger
            onClick={() => handleFingerprintConfirm(false)}
            disabled={fpConfirming}
          >
            {t('share.reject')}
          </BtnDanger>
          <BtnPrimary
            onClick={() => handleFingerprintConfirm(true)}
            disabled={fpConfirming}
          >
            {fpConfirming ? <Spinner /> : <Icon name="check" size={12} color="#020504" />}
            {t('share.confirm')}
          </BtnPrimary>
        </div>
      </>
    );
  }

  function renderFileExport() {
    return (
      <>
        <Breadcrumb path={t('share.crumb.fileExport')} />
        <SectionLabel>{t('share.exportEncrypted')}</SectionLabel>
        <p className="text-[12px] text-tx2 mb-4 leading-[1.6]">
          {t('share.exportHint')}
        </p>
        <div className="flex items-center gap-2 mb-4">
          <span className="w-1.5 h-1.5 rounded-full bg-accent shrink-0" />
          <span className="text-[11px] text-tx2 font-mono">
            {t(selectedIds.length === 1 ? 'share.selectedForExport_one' : 'share.selectedForExport_other', { n: selectedIds.length })}
          </span>
        </div>
        {error && <div className="mb-3"><InlineError msg={error} /></div>}
        <Divider />
        <div className="flex justify-between">
          <BtnSecondary onClick={() => { setStep('method'); setError(''); }}>{t('share.back')}</BtnSecondary>
          <BtnPrimary onClick={handleFileExport} disabled={exportLoading}>
            {exportLoading ? <Spinner /> : <Icon name="export" size={12} color="#020504" />}
            {t('share.saveEncrypted')}
          </BtnPrimary>
        </div>
      </>
    );
  }

  function renderFileImport() {
    return (
      <>
        <Breadcrumb path={t('share.crumb.fileImport')} />
        <SectionLabel>{t('share.importEncrypted')}</SectionLabel>
        <p className="text-[12px] text-tx2 mb-4 leading-[1.6]">
          {t('share.importHint')}
        </p>

        {!importFileReady ? (
          <BtnSecondary onClick={handleFileImportOpen} disabled={importLoading} className="w-full mb-4">
            {importLoading ? <Spinner /> : <Icon name="external" size={12} />}
            {t('share.openPackage')}
          </BtnSecondary>
        ) : (
          <>
            <div className="flex items-center gap-2 mb-3">
              <Icon name="check" size={12} color="oklch(0.70 0.17 162)" />
              <span className="text-[11px] text-accent font-mono">{t('share.fileSelected')}</span>
            </div>
            <div className="mb-4">
              <label className="block text-[10px] font-mono text-tx3 tracking-[0.08em] mb-1.5">
                {t('share.passphrase')}
              </label>
              <input
                type="text"
                value={importPassphrase}
                onChange={(e) => setImportPassphrase(e.target.value)}
                onKeyDown={(e) => { if (e.key === 'Enter') handleFileImportSubmit(); }}
                placeholder="word-word-word-word"
                className={[
                  'w-full h-9 bg-raised border border-bd2 rounded-[3px] px-3',
                  'text-[13px] font-mono text-tx placeholder:text-tx4',
                  'outline-none focus:border-accent transition-colors duration-150',
                ].join(' ')}
              />
            </div>
          </>
        )}

        {error && <div className="mb-3"><InlineError msg={error} /></div>}
        <Divider />
        <div className="flex justify-between">
          <BtnSecondary onClick={() => { setStep('method'); setError(''); setImportFileReady(false); setImportPassphrase(''); }}>
            {t('share.back')}
          </BtnSecondary>
          <BtnPrimary
            onClick={handleFileImportSubmit}
            disabled={!importFileReady || !importPassphrase || importLoading}
          >
            {importLoading ? <Spinner /> : null}
            {t('share.import')}
          </BtnPrimary>
        </div>
      </>
    );
  }

  function renderDoneSend() {
    return (
      <>
        <Breadcrumb path={t('share.crumb.lanSendDone')} />
        <div className="py-6 text-center">
          <div className="w-10 h-10 rounded-full bg-accent-b border border-accent-d flex items-center justify-center mx-auto mb-4">
            <Icon name="check" size={18} color="oklch(0.70 0.17 162)" />
          </div>
          <div className="text-[13px] font-semibold text-tx mb-1">{t('share.transferComplete')}</div>
          <div className="text-[12px] text-tx3 font-mono">
            {t(sentCount === 1 ? 'share.sentSuccess_one' : 'share.sentSuccess_other', { n: sentCount })}
          </div>
        </div>
        <Divider />
        <div className="flex justify-end">
          <BtnPrimary onClick={() => { onSendDone?.(); onClose(); }}>{t('share.close')}</BtnPrimary>
        </div>
      </>
    );
  }

  function renderDoneReceive() {
    return (
      <>
        <Breadcrumb path={t('share.crumb.lanReceiveDone')} />
        <div className="py-4">
          <div className="flex items-center gap-2 mb-3">
            <div className="w-7 h-7 rounded-full bg-accent-b border border-accent-d flex items-center justify-center shrink-0">
              <Icon name="check" size={13} color="oklch(0.70 0.17 162)" />
            </div>
            <div className="text-[13px] font-semibold text-tx">{t('share.transferComplete')}</div>
          </div>
          <div className="text-[11px] text-tx3 font-mono mb-3">
            {t(receivedNames.length === 1 ? 'share.receivedList_one' : 'share.receivedList_other', { n: receivedNames.length })}
          </div>
          <div className="bg-raised border border-bd rounded-[3px] divide-y divide-bd max-h-[120px] overflow-y-auto">
            {receivedNames.map((name, i) => (
              <div key={i} className="px-3 py-2 text-[12px] font-mono text-tx2 flex items-center gap-2">
                <span className="w-1.5 h-1.5 rounded-full bg-accent shrink-0" />
                {name}
              </div>
            ))}
          </div>
        </div>
        <Divider />
        <div className="flex justify-end">
          <BtnPrimary onClick={handleImportDoneClose}>
            <Icon name="check" size={12} color="#020504" />
            {t('share.reloadVault')}
          </BtnPrimary>
        </div>
      </>
    );
  }

  function renderDoneFileExport() {
    return (
      <>
        <Breadcrumb path={t('share.crumb.fileExportDone')} />
        <div className="flex items-center gap-2 mb-4">
          <div className="w-7 h-7 rounded-full bg-accent-b border border-accent-d flex items-center justify-center shrink-0">
            <Icon name="check" size={13} color="oklch(0.70 0.17 162)" />
          </div>
          <div className="text-[13px] font-semibold text-tx">{t('share.fileSaved')}</div>
        </div>

        <SectionLabel>{t('share.passphraseSeparate')}</SectionLabel>
        <div className="bg-raised border border-bd2 rounded-[3px] px-4 py-3 flex items-center gap-3 mb-3">
          <span className="flex-1 font-mono text-[13px] text-accent tracking-[0.05em] select-all break-all">
            {exportPassphrase}
          </span>
          <button
            onClick={handleCopyPassphrase}
            title={t('share.copyPassphrase')}
            className={[
              'flex items-center gap-1.5 border rounded-[3px] px-2 py-1',
              'text-[10px] font-mono tracking-wide shrink-0 transition-all duration-150 cursor-pointer',
              copiedPass
                ? 'bg-accent-b border-accent-d text-accent'
                : 'border-bd2 text-tx3 bg-transparent hover:border-tx3 hover:text-tx',
            ].join(' ')}
          >
            <Icon name={copiedPass ? 'check' : 'copy'} size={11} color={copiedPass ? 'oklch(0.70 0.17 162)' : 'currentColor'} />
            {copiedPass ? t('share.copied') : t('share.copy')}
          </button>
        </div>
        <p className="text-[11px] text-tx3 font-mono leading-[1.5]">
          {t('share.passphraseNote')}
        </p>
        <Divider />
        <div className="flex justify-end">
          <BtnPrimary onClick={onClose}>{t('share.done')}</BtnPrimary>
        </div>
      </>
    );
  }

  function renderDoneFileImport() {
    return (
      <>
        <Breadcrumb path={t('share.crumb.fileImportDone')} />
        <div className="flex items-center gap-2 mb-4">
          <div className="w-7 h-7 rounded-full bg-accent-b border border-accent-d flex items-center justify-center shrink-0">
            <Icon name="check" size={13} color="oklch(0.70 0.17 162)" />
          </div>
          <div className="text-[13px] font-semibold text-tx">{t('share.importComplete')}</div>
        </div>
        <div className="text-[11px] text-tx3 font-mono mb-3">
          {t(importedNames.length === 1 ? 'share.importedList_one' : 'share.importedList_other', { n: importedNames.length })}
        </div>
        <div className="bg-raised border border-bd rounded-[3px] divide-y divide-bd max-h-[120px] overflow-y-auto mb-4">
          {importedNames.map((name, i) => (
            <div key={i} className="px-3 py-2 text-[12px] font-mono text-tx2 flex items-center gap-2">
              <span className="w-1.5 h-1.5 rounded-full bg-accent shrink-0" />
              {name}
            </div>
          ))}
        </div>
        <Divider />
        <div className="flex justify-end">
          <BtnPrimary onClick={handleImportDoneClose}>
            <Icon name="check" size={12} color="#020504" />
            {t('share.reloadVault')}
          </BtnPrimary>
        </div>
      </>
    );
  }

  // ─── Internet relay renderers ──────────────────────────────────────────────

  async function handleRelayRelayLaunch() {
    setRelayLoading(true);
    setError('');
    try {
      const result = await invoke<{ code: string; passphrase: string }>('share_relay_send', { itemIds: selectedIds });
      setRelayCode(result.code);
      setRelayPassphrase(result.passphrase);
      setStep('internet-done-send');
    } catch (e) {
      setError(String(e));
    } finally {
      setRelayLoading(false);
    }
  }

  async function handleRelayReceive() {
    if (!relayRxCode || !relayRxPass) return;
    setRelayRxLoading(true);
    setError('');
    try {
      const names = await invoke<string[]>('share_relay_receive', { code: relayRxCode.toUpperCase(), passphrase: relayRxPass });
      setRelayRxNames(names);
      setStep('internet-done-receive');
    } catch (e) {
      setError(String(e));
    } finally {
      setRelayRxLoading(false);
    }
  }

  function renderInternetSend() {
    return (
      <>
        <Breadcrumb path={t('share.crumb.internetSend')} />
        <SectionLabel>{t('share.sendViaRelay')}</SectionLabel>
        <p className="text-[12px] text-tx2 mb-3 leading-[1.6]">
          {t('share.relayIntro')}{' '}
          {t('share.validFor')} <span className="text-accent font-mono">{t('share.hours24')}</span> {t('share.retrievedOnce')}
        </p>
        <div className="bg-raised border border-bd2 rounded-[3px] px-3 py-2 text-[10px] text-tx3 font-mono leading-[1.7] mb-3 flex items-start gap-2">
          <Icon name="shield" size={11} color="currentColor" />
          <span>{t('share.relayNotice')}</span>
        </div>
        <div className="flex items-center gap-2 mb-4">
          <span className="w-1.5 h-1.5 rounded-full bg-accent shrink-0" />
          <span className="text-[11px] text-tx2 font-mono">
            {t('share.sending')} <span className="text-accent font-bold">{selectedIds.length}</span> {t(selectedIds.length === 1 ? 'share.itemWord_one' : 'share.itemWord_other')}
          </span>
        </div>
        {error && <div className="mb-3"><InlineError msg={error} /></div>}
        <Divider />
        <div className="flex justify-between">
          <BtnSecondary onClick={() => { setStep('method'); setError(''); }}>{t('share.back')}</BtnSecondary>
          <BtnPrimary onClick={handleRelayRelayLaunch} disabled={relayLoading || selectedIds.length === 0}>
            {relayLoading ? <Spinner /> : <Icon name="shield" size={12} color="#020504" />}
            {relayLoading ? t('share.uploading') : t('share.uploadGetCode')}
          </BtnPrimary>
        </div>
      </>
    );
  }

  function renderInternetReceive() {
    return (
      <>
        <Breadcrumb path={t('share.crumb.internetReceive')} />
        <SectionLabel>{t('share.enterRelay')}</SectionLabel>
        <div className="bg-raised border border-bd2 rounded-[3px] px-3 py-2 text-[10px] text-tx3 font-mono leading-[1.7] mb-3 flex items-start gap-2">
          <Icon name="shield" size={11} color="currentColor" />
          <span>{t('share.relayNotice')}</span>
        </div>
        <div className="mb-3">
          <label className="block text-[10px] font-mono text-tx3 tracking-[0.08em] mb-1.5">{t('share.code')}</label>
          <input
            type="text"
            value={relayRxCode}
            onChange={(e) => setRelayRxCode(e.target.value.toUpperCase())}
            placeholder="XXXX-XXXX"
            maxLength={9}
            className="w-full h-9 bg-raised border border-bd2 rounded-[3px] px-3 text-[15px] font-mono text-accent tracking-[0.2em] placeholder:text-tx3 outline-none focus:border-accent transition-colors"
          />
        </div>
        <div className="mb-4">
          <label className="block text-[10px] font-mono text-tx3 tracking-[0.08em] mb-1.5">{t('share.passphrase')}</label>
          <input
            type="text"
            value={relayRxPass}
            onChange={(e) => setRelayRxPass(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Enter') handleRelayReceive(); }}
            placeholder="abc123XYZ…"
            className="w-full h-9 bg-raised border border-bd2 rounded-[3px] px-3 text-[13px] font-mono text-tx placeholder:text-tx3 outline-none focus:border-accent transition-colors"
          />
        </div>
        {error && <div className="mb-3"><InlineError msg={error} /></div>}
        <Divider />
        <div className="flex justify-between">
          <BtnSecondary onClick={() => { setStep('method'); setError(''); }}>{t('share.back')}</BtnSecondary>
          <BtnPrimary onClick={handleRelayReceive} disabled={relayRxLoading || !relayRxCode || !relayRxPass}>
            {relayRxLoading ? <Spinner /> : null}
            {relayRxLoading ? t('share.downloading') : t('share.import')}
          </BtnPrimary>
        </div>
      </>
    );
  }

  function renderInternetDoneSend() {
    return (
      <>
        <Breadcrumb path={t('share.crumb.internetSendDone')} />
        <div className="flex items-center gap-2 mb-4">
          <div className="w-7 h-7 rounded-full bg-accent-b border border-accent-d flex items-center justify-center shrink-0">
            <Icon name="check" size={13} color="oklch(0.70 0.17 162)" />
          </div>
          <div className="text-[13px] font-semibold text-tx">{t('share.uploaded')}</div>
        </div>

        <SectionLabel>{t('share.sendBoth')}</SectionLabel>

        <RelayCodeDisplay code={relayCode} passphrase={relayPassphrase} />

        <Divider />
        <div className="flex justify-end">
          <BtnPrimary onClick={() => { onSendDone?.(); onClose(); }}>{t('share.done')}</BtnPrimary>
        </div>
      </>
    );
  }

  function renderInternetDoneReceive() {
    return (
      <>
        <Breadcrumb path={t('share.crumb.internetReceiveDone')} />
        <div className="flex items-center gap-2 mb-4">
          <div className="w-7 h-7 rounded-full bg-accent-b border border-accent-d flex items-center justify-center shrink-0">
            <Icon name="check" size={13} color="oklch(0.70 0.17 162)" />
          </div>
          <div className="text-[13px] font-semibold text-tx">{t('share.importComplete')}</div>
        </div>
        <div className="text-[11px] text-tx3 font-mono mb-3">
          {t(relayRxNames.length === 1 ? 'share.importedFromRelay_one' : 'share.importedFromRelay_other', { n: relayRxNames.length })}
        </div>
        <div className="bg-raised border border-bd rounded-[3px] divide-y divide-bd max-h-[120px] overflow-y-auto mb-4">
          {relayRxNames.map((name, i) => (
            <div key={i} className="px-3 py-2 text-[12px] font-mono text-tx2 flex items-center gap-2">
              <span className="w-1.5 h-1.5 rounded-full bg-accent shrink-0" />
              {name}
            </div>
          ))}
        </div>
        <Divider />
        <div className="flex justify-end">
          <BtnPrimary onClick={handleImportDoneClose}>
            <Icon name="check" size={12} color="#020504" />
            {t('share.reloadVault')}
          </BtnPrimary>
        </div>
      </>
    );
  }

  function renderFailed() {
    return (
      <>
        <Breadcrumb path={t('share.crumb.error')} />
        <div className="py-6 text-center">
          <div className="w-10 h-10 rounded-full bg-danger-b border border-danger flex items-center justify-center mx-auto mb-4">
            <Icon name="close" size={18} color="oklch(0.62 0.20 22)" />
          </div>
          <div className="text-[13px] font-semibold text-tx mb-2">{t('share.transferFailed')}</div>
          <div className="text-[12px] text-danger font-mono bg-danger-b border border-danger rounded-[3px] px-3 py-2 mt-2 text-left">
            {failedError}
          </div>
        </div>
        <Divider />
        <div className="flex justify-between">
          <BtnSecondary onClick={onClose}>{t('share.close')}</BtnSecondary>
          <BtnPrimary onClick={() => { setStep('method'); setMethod(null); setError(''); setFailedError(''); }}>
            {t('share.retry')}
          </BtnPrimary>
        </div>
      </>
    );
  }

  // ------------------------------------------------------------------
  // Body dispatch
  // ------------------------------------------------------------------

  function renderBody() {
    switch (step) {
      case 'method':          return renderMethod();
      case 'lan-send':        return renderLanSend();
      case 'lan-receive':     return renderLanReceive();
      case 'fingerprint':     return renderFingerprint();
      case 'file-export':     return renderFileExport();
      case 'file-import':     return renderFileImport();
      case 'done-send':       return renderDoneSend();
      case 'done-receive':    return renderDoneReceive();
      case 'done-file-export':return renderDoneFileExport();
      case 'done-file-import':    return renderDoneFileImport();
      case 'internet-send':       return renderInternetSend();
      case 'internet-receive':    return renderInternetReceive();
      case 'internet-done-send':  return renderInternetDoneSend();
      case 'internet-done-receive': return renderInternetDoneReceive();
      case 'failed':              return renderFailed();
      default:                    return null;
    }
  }

  // ------------------------------------------------------------------
  // Outer shell
  // ------------------------------------------------------------------

  return (
    <div className="absolute inset-0 bg-black/70 flex items-center justify-center z-20 p-5">
      <div className="bg-surface border border-bd rounded-[4px] p-4 w-full animate-fade-in">
        {/* Modal header */}
        <div className="flex items-center justify-between mb-4">
          <div className="flex items-center gap-2">
            <Icon name="shield" size={14} color="oklch(0.70 0.17 162)" />
            <span className="text-[12px] font-bold tracking-wider font-ui text-tx">{t('share.title')}</span>
          </div>
          <button
            onClick={step === 'method' ? onClose : handleCancel}
            className="flex items-center justify-center w-6 h-6 rounded-[3px] text-tx3 hover:text-tx hover:bg-raised transition-all duration-150 cursor-pointer border-none bg-transparent"
          >
            <Icon name="close" size={13} />
          </button>
        </div>

        {/* Step content */}
        {renderBody()}
      </div>
    </div>
  );
}
