import { Icon } from '../ui/Icon';
import { useVaultStore } from '../../store';
import { copyPlain } from '../../lib/clipboard';
import { useWslStore } from '../../store/wslStore';
import { formatWslError, useWslConfigure, useWslDetect, useWslRemove } from '../../hooks/useWsl';
import { t as tr, useTranslation } from '../../i18n';
import type { WslActionReport, WslDistro } from '../../types';

export const WSLCONFIG_SNIPPET = `[wsl2]
networkingMode=mirrored`;

const BTN =
  'h-7 px-3 bg-transparent border border-bd2 rounded-[3px] text-tx2 text-[11px] cursor-pointer font-ui font-semibold tracking-[0.06em] hover:text-tx transition-colors disabled:opacity-40';

/**
 * Settings → WSL Integration. Rendered only on Windows (`isWindows`) and only
 * once detection reports at least one distro. Configure / Remove delegate to
 * the `setup wsl` contract inside the distro; `.wslconfig` is never written —
 * mirrored networking is shown as copy-only guidance.
 */
export function WslIntegrationSection({ isWindows }: { isWindows: boolean }) {
  const detect = useWslDetect(isWindows);
  const { t } = useTranslation();

  if (!isWindows) return null;
  if (detect.isPending) return null;

  if (detect.isError) {
    return (
      <>
        <Header onRefresh={() => detect.refetch()} refreshing={detect.isFetching} />
        <div role="status" className="mt-2 mb-2 px-3 py-2 rounded-[3px] border border-bd bg-raised text-[12px] font-mono text-tx3">
          {t('wsl.detectFailed', { error: formatWslError(detect.error) })}
        </div>
      </>
    );
  }

  const status = detect.data;
  if (!status.available) return null;

  return (
    <>
      <Header onRefresh={() => detect.refetch()} refreshing={detect.isFetching} />
      {!status.mirrored && <MirroredBanner />}
      <div className="mt-1">
        {status.distros.map((d) => <DistroRow key={d.name} distro={d} />)}
      </div>
      <ReportPanel />
    </>
  );
}

function Header({ onRefresh, refreshing }: { onRefresh: () => void; refreshing: boolean }) {
  const { t } = useTranslation();
  return (
    <div className="mt-8 mb-1">
      <div className="flex items-center text-[14px] font-semibold text-tx2 tracking-[0.1em] font-ui pb-2 border-b border-bd">
        <span className="flex-1">{t('wsl.title')}</span>
        <button
          onClick={onRefresh}
          disabled={refreshing}
          className="text-tx3 hover:text-tx transition-colors disabled:opacity-40"
          title={t('wsl.refresh')}
          aria-label={t('wsl.refreshAria')}
        >
          <Icon name="refresh" size={13} />
        </button>
      </div>
    </div>
  );
}

function MirroredBanner() {
  const showToast = useVaultStore((s) => s.showToast);
  const { t } = useTranslation();
  // Split on placeholders so the inline <code> spans survive translation.
  const note = t('wsl.mirroredNote').split(/(\{cmd\}|\{shutdown\})/).map((part, i) =>
    part === '{cmd}' ? <code key={i}>crypt-env</code>
      : part === '{shutdown}' ? <code key={i}>wsl --shutdown</code>
      : part,
  );
  return (
    <div data-testid="wsl-mirrored-banner" className="mt-2 mb-2 rounded-[3px] border border-bd bg-raised overflow-hidden">
      <div className="flex items-center justify-between px-3 py-2 border-b border-bd">
        <span className="text-[11px] font-mono text-tx3 tracking-[0.06em]">
          {t('wsl.addTo')}
        </span>
        <button
          onClick={() => copyPlain(WSLCONFIG_SNIPPET).then(() => showToast(t('wsl.snippetCopied')))}
          className="text-tx3 hover:text-accent transition-colors"
          title={t('wsl.copySnippet')}
          aria-label={t('wsl.copySnippetAria')}
        >
          <Icon name="copy" size={12} />
        </button>
      </div>
      <pre className="text-[11px] font-mono text-tx2 px-4 pt-3 pb-2 leading-[1.6]">{WSLCONFIG_SNIPPET}</pre>
      <p className="px-4 pb-3 text-[12px] font-mono text-tx3 leading-[1.6]">
        {note}
      </p>
    </div>
  );
}

function DistroRow({ distro }: { distro: WslDistro }) {
  const showToast      = useVaultStore((s) => s.showToast);
  const selected       = useWslStore((s) => s.selectedDistro === distro.name);
  const selectDistro   = useWslStore((s) => s.selectDistro);
  const setLastAction  = useWslStore((s) => s.setLastAction);
  const configure      = useWslConfigure();
  const remove         = useWslRemove();
  const busy           = configure.isPending || remove.isPending;
  const { t }          = useTranslation();

  const run = (action: 'configure' | 'remove') => {
    selectDistro(distro.name);
    setLastAction(null);
    const m = action === 'configure' ? configure : remove;
    m.mutate(distro.name, {
      onSuccess: (report) => setLastAction({ distro: distro.name, action, report }),
      onError:   (e) => showToast(formatWslError(e), 'error'),
    });
  };

  return (
    <div
      data-testid={`wsl-distro-${distro.name}`}
      className={[
        'flex items-center gap-3 min-h-[44px] py-2 border-b border-bd',
        selected ? 'bg-surface' : '',
      ].join(' ')}
    >
      <span className="text-tx3 shrink-0"><Icon name="terminal" size={14} /></span>
      <div className="flex-1 min-w-0">
        <div className="text-[13px] font-medium text-tx font-ui truncate">{distro.name}</div>
        <div className="text-[11px] font-mono text-tx3 truncate">
          {distro.defaultUser ?? t('wsl.unknownUser')} ·{' '}
          <span className={distro.configured && distro.launcher ? 'text-accent' : ''}>
            {!distro.configured
              ? t('wsl.notConfigured')
              : distro.launcher
                ? t('wsl.configured')
                : t('wsl.configuredNoLauncher')}
          </span>
        </div>
      </div>
      <button onClick={() => run('configure')} disabled={busy} className={BTN}>
        {configure.isPending ? t('wsl.configuring') : distro.configured ? t('wsl.reconfigure') : t('wsl.configure')}
      </button>
      {distro.configured && (
        <button onClick={() => run('remove')} disabled={busy} className={BTN}>
          {remove.isPending ? t('wsl.removing') : t('wsl.remove')}
        </button>
      )}
    </div>
  );
}

function ReportPanel() {
  const last = useWslStore((s) => s.lastAction);
  const { t } = useTranslation();
  if (!last) return null;
  const { distro, action, report } = last;

  const lines: string[] = [];
  if (action === 'configure') {
    lines.push(t('wsl.report.wrote', { path: report.env_file }));
    if (report.rc_files.length === 0) lines.push(t('wsl.report.shellBlockPresent'));
    report.rc_files.forEach((rc) => lines.push(t('wsl.report.addedBlock', { path: rc })));
    report.backups.forEach((b) => lines.push(t('wsl.report.backupWritten', { path: b })));
  } else {
    const launcherDeleted = report.launcher_status === 'deleted';
    if (!report.env_file_changed && report.rc_files.length === 0 && !launcherDeleted) lines.push(t('wsl.report.nothingToRemove'));
    report.rc_files.forEach((rc) => lines.push(t('wsl.report.removedBlock', { path: rc })));
    if (report.env_file_changed) lines.push(t('wsl.report.deleted', { path: report.env_file }));
  }
  const launcher = launcherLine(report);
  if (launcher) lines.push(launcher);

  return (
    <div data-testid="wsl-report" className="mt-2 mb-2 px-3 py-2.5 rounded-[3px] border border-bd bg-raised">
      <div className="text-[12px] font-semibold text-tx font-ui mb-1">
        {action === 'configure' ? t('wsl.report.configuredDistro', { distro }) : t('wsl.report.removedFrom', { distro })}
      </div>
      <ul className="text-[11px] font-mono text-tx3 leading-[1.8]">
        {lines.map((l) => <li key={l}>{l}</li>)}
      </ul>
      {action === 'configure' && (
        <div className="text-[11px] font-mono text-tx2 mt-1">{t('wsl.report.openNewTerminal')}</div>
      )}
    </div>
  );
}

/** Report line for the managed `crypt-env` launcher, if it was involved. */
export function launcherLine(report: WslActionReport): string | null {
  const path = report.launcher ?? tr('wsl.launcher.default');
  switch (report.launcher_status) {
    case 'written':   return tr('wsl.launcher.written', { path });
    case 'unchanged': return tr('wsl.launcher.unchanged', { path });
    case 'deleted':   return tr('wsl.launcher.deleted', { path });
    case 'skipped':   return tr('wsl.launcher.skipped', { reason: report.launcher_note ?? tr('wsl.launcher.unknownReason') });
    default:          return null;
  }
}
