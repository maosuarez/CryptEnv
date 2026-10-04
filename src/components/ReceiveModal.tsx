import { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Icon } from './ui/Icon';
import { useVaultStore } from '../store';
import { useTranslation } from '../i18n';

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
}

type Step = 'lan-receive' | 'fingerprint' | 'done-receive' | 'file-import' | 'done-file-import' | 'failed';

interface ReceiveModalProps {
  onClose: () => void;
}

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

function InlineError({ msg }: { msg: string }) {
  return (
    <div className="text-[12px] text-danger font-mono bg-danger-b border border-danger rounded-[3px] px-3 py-2">
      {msg}
    </div>
  );
}

function Breadcrumb({ path }: { path: string }) {
  return (
    <div className="text-[10px] font-mono text-tx3 tracking-[0.08em] mb-4">{path}</div>
  );
}

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

function FingerprintDisplay({ fp }: { fp: string }) {
  return (
    <div className="bg-raised border border-bd2 rounded-[3px] px-4 py-3 text-center my-3">
      <span className="text-[16px] font-mono text-accent tracking-[0.08em] select-all">
        {fp}
      </span>
    </div>
  );
}

export function ReceiveModal({ onClose }: ReceiveModalProps) {
  const go = useVaultStore((s) => s.go);
  const { t } = useTranslation();
  const [step, setStep] = useState<Step>('lan-receive');
  const [inputCode, setInputCode] = useState('');
  const [receiveStatus, setReceiveStatus] = useState<'idle' | 'loading' | 'connecting'>('idle');
  const [fingerprint, setFingerprint] = useState('');
  const [fpConfirming, setFpConfirming] = useState(false);
  const [receivedNames, setReceivedNames] = useState<string[]>([]);
  const [error, setError] = useState('');
  const [failedError, setFailedError] = useState('');
  const [importPassphrase, setImportPassphrase] = useState('');
  const [importLoading, setImportLoading] = useState(false);
  const [importedNames, setImportedNames] = useState<string[]>([]);
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
        if (result.state === 'awaiting_fingerprint' && result.fingerprint) {
          setFingerprint(result.fingerprint);
          stopPolling();
          setStep('fingerprint');
        } else if (result.state === 'done') {
          stopPolling();
          if (result.receivedNames && result.receivedNames.length > 0) {
            setReceivedNames(result.receivedNames);
            setStep('done-receive');
          }
        } else if (result.state === 'failed') {
          stopPolling();
          setFailedError(result.error ?? t('receive.transferFailed'));
          setStep('failed');
        } else if (result.state === 'cancelled') {
          stopPolling();
          onClose();
        }
      } catch (e) {
        stopPolling();
        setFailedError(String(e));
        setStep('failed');
      }
    }, 800);
  }, [stopPolling, onClose, t]);

  const handleCancel = async () => {
    stopPolling();
    try { await invoke('share_cancel'); } catch { /* best effort */ }
    onClose();
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
      startPolling();
    } catch (e) {
      setError(String(e));
    } finally {
      setFpConfirming(false);
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

  const handleImportDoneClose = () => {
    onClose();
    go('vault');
  };

  function renderLanReceive() {
    const isConnecting = receiveStatus === 'connecting';
    const isLoading = receiveStatus === 'loading';

    return (
      <>
        <Breadcrumb path={t('receive.crumb.lanReceive')} />
        <SectionLabel>{t('receive.enterPairingCode')}</SectionLabel>
        <PairingCodeInput
          value={inputCode}
          onChange={setInputCode}
          onComplete={() => { if (!isConnecting && !isLoading) handleReceiveSubmit(); }}
        />
        {isConnecting && (
          <div className="flex items-center justify-center gap-2 text-[11px] text-tx3 font-mono mb-2">
            <Spinner />
            <span>{t('receive.connecting')}</span>
          </div>
        )}
        {error && <div className="mb-3"><InlineError msg={error} /></div>}
        <Divider />
        <div className="flex justify-between">
          <BtnSecondary onClick={handleCancel}>{t('receive.back')}</BtnSecondary>
          <BtnPrimary
            onClick={handleReceiveSubmit}
            disabled={inputCode.length < 6 || isLoading || isConnecting}
          >
            {isLoading ? <Spinner /> : null}
            {t('receive.connect')}
          </BtnPrimary>
        </div>
      </>
    );
  }

  function renderFingerprint() {
    return (
      <>
        <Breadcrumb path={t('receive.crumb.lanVerify')} />
        <SectionLabel>{t('receive.verifyFingerprint')}</SectionLabel>
        <p className="text-[12px] text-tx2 mb-3 leading-[1.6]">
          {t('receive.verifyPrompt')}
        </p>
        <FingerprintDisplay fp={fingerprint} />
        <p className="text-[11px] text-tx3 font-mono mt-2 mb-4 leading-[1.5]">
          {t('receive.verifyHint')}
        </p>
        {error && <div className="mb-3"><InlineError msg={error} /></div>}
        <Divider />
        <div className="flex gap-2 justify-between">
          <BtnSecondary onClick={() => handleFingerprintConfirm(false)} disabled={fpConfirming}>
            {t('receive.reject')}
          </BtnSecondary>
          <BtnPrimary
            onClick={() => handleFingerprintConfirm(true)}
            disabled={fpConfirming}
          >
            {fpConfirming ? <Spinner /> : <Icon name="check" size={12} color="#020504" />}
            {t('receive.confirm')}
          </BtnPrimary>
        </div>
      </>
    );
  }

  function renderDoneReceive() {
    return (
      <>
        <Breadcrumb path={t('receive.crumb.lanDone')} />
        <SectionLabel>{t('receive.receivedItems')}</SectionLabel>
        <p className="text-[12px] text-tx2 mb-3 leading-[1.6]">
          {t(receivedNames.length === 1 ? 'receive.received_one' : 'receive.received_other', { n: receivedNames.length })}
        </p>
        <div className="bg-raised border border-bd rounded-[3px] px-3 py-2 mb-4 max-h-[200px] overflow-y-auto">
          {receivedNames.map((name, i) => (
            <div key={i} className="text-[11px] text-tx2 font-mono py-1 border-b border-bd last:border-b-0">
              {name}
            </div>
          ))}
        </div>
        <Divider />
        <div className="flex justify-end">
          <BtnPrimary onClick={handleImportDoneClose}>{t('receive.close')}</BtnPrimary>
        </div>
      </>
    );
  }

  function renderFileImport() {
    return (
      <>
        <Breadcrumb path={t('receive.crumb.fileImport')} />
        <SectionLabel>{t('receive.importFile')}</SectionLabel>
        <p className="text-[12px] text-tx2 mb-4 leading-[1.6]">
          {t('receive.importFileHint')}
        </p>
        <div className="mb-3">
          <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.06em] mb-1">{t('receive.passphrase')}</div>
          <input
            type="password"
            value={importPassphrase}
            onChange={(e) => setImportPassphrase(e.target.value)}
            placeholder={t('receive.passphrasePlaceholder')}
            className="w-full bg-bg border border-bd2 text-tx font-mono text-[13px] rounded-[3px] px-3 py-[7px] outline-none focus:border-accent-d transition-colors"
          />
        </div>
        {error && <div className="mb-3"><InlineError msg={error} /></div>}
        <Divider />
        <div className="flex gap-2 justify-between">
          <BtnSecondary onClick={handleCancel}>{t('common.cancel')}</BtnSecondary>
          <BtnPrimary onClick={handleFileImportSubmit} disabled={!importPassphrase || importLoading}>
            {importLoading ? <Spinner /> : null}
            {t('receive.import')}
          </BtnPrimary>
        </div>
      </>
    );
  }

  function renderDoneFileImport() {
    return (
      <>
        <Breadcrumb path={t('receive.crumb.fileImportDone')} />
        <SectionLabel>{t('receive.importSuccess')}</SectionLabel>
        <p className="text-[12px] text-tx2 mb-3 leading-[1.6]">
          {t(importedNames.length === 1 ? 'receive.imported_one' : 'receive.imported_other', { n: importedNames.length })}
        </p>
        <div className="bg-raised border border-bd rounded-[3px] px-3 py-2 mb-4 max-h-[200px] overflow-y-auto">
          {importedNames.map((name, i) => (
            <div key={i} className="text-[11px] text-tx2 font-mono py-1 border-b border-bd last:border-b-0">
              {name}
            </div>
          ))}
        </div>
        <Divider />
        <div className="flex justify-end">
          <BtnPrimary onClick={handleImportDoneClose}>{t('receive.close')}</BtnPrimary>
        </div>
      </>
    );
  }

  function renderFailed() {
    return (
      <>
        <Breadcrumb path={t('receive.crumb.error')} />
        <SectionLabel>{t('receive.transferFailed')}</SectionLabel>
        <InlineError msg={failedError} />
        <Divider />
        <div className="flex justify-end">
          <BtnSecondary onClick={handleCancel}>{t('receive.close')}</BtnSecondary>
        </div>
      </>
    );
  }

  function renderBody() {
    switch (step) {
      case 'lan-receive': return renderLanReceive();
      case 'fingerprint': return renderFingerprint();
      case 'done-receive': return renderDoneReceive();
      case 'file-import': return renderFileImport();
      case 'done-file-import': return renderDoneFileImport();
      case 'failed': return renderFailed();
    }
  }

  return (
    <div className="absolute inset-0 bg-black/70 flex items-center justify-center z-20 p-5">
      <div className="bg-surface border border-bd rounded-[4px] p-4 w-full animate-fade-in">
        {/* Modal header */}
        <div className="flex items-center justify-between mb-4">
          <div className="flex items-center gap-2">
            <Icon name="shield" size={14} color="oklch(0.70 0.17 162)" />
            <span className="text-[12px] font-bold tracking-wider font-ui text-tx">{t('receive.title')}</span>
          </div>
          <button
            onClick={step === 'lan-receive' ? onClose : handleCancel}
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
