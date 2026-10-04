import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Icon } from './ui/Icon';
import { useTranslation } from '../i18n';
import { copySecret } from '../lib/clipboard';

interface SetupWizardProps {
  onClose: () => void;
}

type Step = 'welcome' | 'mcp';

export function SetupWizard({ onClose }: SetupWizardProps) {
  const [step,        setStep]        = useState<Step>('welcome');
  const [token,       setToken]       = useState<string | null>(null);
  const [targetPath,  setTargetPath]  = useState('~/.mcp.json');
  const [generating,  setGenerating]  = useState(false);
  const [writtenPath, setWrittenPath] = useState<string | null>(null);
  const [error,       setError]       = useState<string | null>(null);
  const [copied,      setCopied]      = useState(false);
  const { t } = useTranslation();

  useEffect(() => {
    if (step === 'mcp') {
      invoke<string | null>('vault_get_mcp_token')
        .then((t) => {
          if (t) {
            setToken(t);
          } else {
            invoke<string>('vault_generate_mcp_token')
              .then(setToken)
              .catch((e) => setError(String(e)));
          }
        })
        .catch((e) => setError(String(e)));
    }
  }, [step]);

  const handleSkip = async () => {
    try { await invoke('app_complete_setup'); } catch { /* best-effort */ }
    onClose();
  };

  const handleGenerate = async () => {
    if (!token) return;
    setGenerating(true);
    setError(null);
    setWrittenPath(null);
    try {
      const path = await invoke<string>('app_generate_mcp_config', { targetPath });
      setWrittenPath(path);
    } catch (e) {
      setError(String(e));
    } finally {
      setGenerating(false);
    }
  };

  const handleDone = async () => {
    try { await invoke('app_complete_setup'); } catch { /* best-effort */ }
    onClose();
  };

  return (
    <div
      className="fixed inset-0 bg-[rgba(4,5,6,.88)] flex items-center justify-center z-[9000] backdrop-blur-[4px]"
      onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}
    >
      <div className="bg-surface border border-bd2 rounded-[4px] p-6 w-[420px] shadow-[0_16px_48px_rgba(0,0,0,.8)]">
        {/* Header */}
        <div className="flex items-center justify-between mb-5">
          <div className="flex items-center gap-2">
            <Icon name="shield" size={15} color="oklch(0.70 0.17 162)" />
            <span className="text-[13px] font-semibold text-tx tracking-[0.03em]">
              {step === 'welcome' ? t('setup.welcomeTitle') : t('setup.claudeTitle')}
            </span>
          </div>
          <div className="flex items-center gap-1.5">
            <div className={`w-1.5 h-1.5 rounded-full ${step === 'welcome' ? 'bg-accent' : 'bg-bd2'}`} />
            <div className={`w-1.5 h-1.5 rounded-full ${step === 'mcp' ? 'bg-accent' : 'bg-bd2'}`} />
          </div>
        </div>

        {step === 'welcome' && (
          <>
            <p className="text-[12px] text-tx2 leading-[1.8] mb-6">
              {t('setup.intro')}
            </p>
            <div className="bg-raised border border-bd rounded-[3px] p-3 mb-6">
              <div className="text-[10px] font-mono text-tx3 tracking-[0.06em] mb-1">{t('setup.howItWorks')}</div>
              <ul className="text-[11px] text-tx2 leading-[2] list-none space-y-0.5">
                <li>{t('setup.how1')}</li>
                <li>{t('setup.how2')}</li>
                <li>{t('setup.how3')}</li>
              </ul>
            </div>
            <div className="flex gap-2">
              <button
                onClick={() => setStep('mcp')}
                className="flex-1 h-[42px] rounded-[3px] border-none text-[12px] font-bold tracking-[0.06em] font-ui bg-accent text-[#020504] cursor-pointer hover:opacity-90 transition-opacity flex items-center justify-center gap-1.5"
              >
                <Icon name="key" size={12} color="#020504" />
                {t('setup.setupClaude')}
              </button>
              <button
                onClick={handleSkip}
                className="px-4 h-[42px] rounded-[3px] border border-bd2 text-[12px] font-semibold text-tx2 bg-transparent cursor-pointer hover:text-tx hover:border-bd transition-colors font-ui tracking-[0.04em]"
              >
                {t('setup.skip')}
              </button>
            </div>
          </>
        )}

        {step === 'mcp' && (
          <>
            <div className="mb-4">
              <div className="text-[10px] font-mono text-tx3 tracking-[0.06em] mb-1.5">{t('setup.mcpAuthToken')}</div>
              <div className="flex items-center gap-2 bg-raised border border-bd rounded-[3px] px-3 py-2">
                <span className="flex-1 text-[11px] font-mono text-tx2 truncate">
                  {token ? '••••••••••••••••' : t('setup.generating')}
                </span>
                {token && (
                  <button
                    onClick={() => {
                      copySecret(token).then(() => {
                        setCopied(true);
                        setTimeout(() => setCopied(false), 2000);
                      }).catch(() => {});
                    }}
                    className="text-[10px] font-mono text-tx3 hover:text-tx transition-colors cursor-pointer bg-transparent border-none px-0"
                  >
                    {copied ? t('setup.copied') : t('setup.copy')}
                  </button>
                )}
                <Icon name="key" size={11} color="oklch(0.72 0.16 68)" />
              </div>
              <div className="text-[10px] text-tx3 font-mono mt-1">{t('setup.storedInVault')}</div>
            </div>

            <div className="mb-5">
              <div className="text-[10px] font-mono text-tx3 tracking-[0.06em] mb-1.5">{t('setup.outputPath')}</div>
              <input
                type="text"
                value={targetPath}
                onChange={(e) => { setTargetPath(e.target.value); setWrittenPath(null); }}
                placeholder="~/.mcp.json"
                className="w-full px-3 py-[7px] bg-raised border border-bd2 rounded-[3px] text-[12px] font-mono text-tx focus:border-accent-d outline-none transition-colors"
              />
              <div className="text-[10px] text-tx3 font-mono mt-1">
                {t('setup.pathHint')}
              </div>
            </div>

            {writtenPath && (
              <div className="mb-4 flex items-start gap-2 bg-[rgba(112,209,162,0.06)] border border-[rgba(112,209,162,0.2)] rounded-[3px] px-3 py-2">
                <Icon name="check" size={12} color="oklch(0.70 0.17 162)" />
                <div>
                  <div className="text-[11px] text-accent font-semibold">{t('setup.written')}</div>
                  <div className="text-[10px] font-mono text-tx3 mt-0.5 break-all">{writtenPath}</div>
                </div>
              </div>
            )}
            {error && (
              <div className="mb-4 text-[11px] text-danger font-mono bg-[rgba(255,80,80,0.06)] border border-[rgba(255,80,80,0.2)] rounded-[3px] px-3 py-2">
                // {error}
              </div>
            )}

            <div className="flex gap-2">
              <button
                onClick={handleGenerate}
                disabled={generating || !token}
                className={[
                  'flex-1 h-[42px] rounded-[3px] border-none text-[12px] font-bold tracking-[0.06em] font-ui',
                  'flex items-center justify-center gap-1.5 transition-all',
                  generating || !token
                    ? 'bg-accent-d text-[#020504] cursor-not-allowed opacity-70'
                    : 'bg-accent text-[#020504] cursor-pointer hover:opacity-90',
                ].join(' ')}
              >
                {generating ? (
                  <><div className="w-3 h-3 rounded-full border-2 border-transparent border-t-[#020504] animate-spin-fast" />{t('setup.writing')}</>
                ) : (
                  <><Icon name="export" size={12} color="#020504" />{t('setup.generate')}</>
                )}
              </button>
              <button
                onClick={handleDone}
                className="px-4 h-[42px] rounded-[3px] border border-bd2 text-[12px] font-semibold text-tx2 bg-transparent cursor-pointer hover:text-tx hover:border-bd transition-colors font-ui tracking-[0.04em]"
              >
                {t('common.done')}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
