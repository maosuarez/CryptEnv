import { Icon } from '../ui/Icon';
import { useVaultStore } from '../../store';
import { useWslStore } from '../../store/wslStore';
import { formatWslError, useWslConfigure, useWslDetect, useWslRemove } from '../../hooks/useWsl';
import type { WslDistro } from '../../types';

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

  if (!isWindows) return null;
  if (detect.isPending) return null;

  if (detect.isError) {
    return (
      <>
        <Header onRefresh={() => detect.refetch()} refreshing={detect.isFetching} />
        <div role="status" className="mt-2 mb-2 px-3 py-2 rounded-[3px] border border-bd bg-raised text-[12px] font-mono text-tx3">
          Could not detect WSL — {formatWslError(detect.error)}
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
  return (
    <div className="mt-8 mb-1">
      <div className="flex items-center text-[14px] font-semibold text-tx2 tracking-[0.1em] font-ui pb-2 border-b border-bd">
        <span className="flex-1">WSL INTEGRATION</span>
        <button
          onClick={onRefresh}
          disabled={refreshing}
          className="text-tx3 hover:text-tx transition-colors disabled:opacity-40"
          title="Refresh"
          aria-label="Refresh WSL detection"
        >
          <Icon name="refresh" size={13} />
        </button>
      </div>
    </div>
  );
}

function MirroredBanner() {
  const showToast = useVaultStore((s) => s.showToast);
  return (
    <div data-testid="wsl-mirrored-banner" className="mt-2 mb-2 rounded-[3px] border border-bd bg-raised overflow-hidden">
      <div className="flex items-center justify-between px-3 py-2 border-b border-bd">
        <span className="text-[11px] font-mono text-tx3 tracking-[0.06em]">
          Add to %USERPROFILE%\.wslconfig
        </span>
        <button
          onClick={() => navigator.clipboard.writeText(WSLCONFIG_SNIPPET).then(() => showToast('Snippet copied'))}
          className="text-tx3 hover:text-accent transition-colors"
          title="Copy snippet"
          aria-label="Copy .wslconfig snippet"
        >
          <Icon name="copy" size={12} />
        </button>
      </div>
      <pre className="text-[11px] font-mono text-tx2 px-4 pt-3 pb-2 leading-[1.6]">{WSLCONFIG_SNIPPET}</pre>
      <p className="px-4 pb-3 text-[12px] font-mono text-tx3 leading-[1.6]">
        Mirrored networking lets <code>crypt-env</code> inside WSL reach the vault on 127.0.0.1.
        Applying it requires <code>wsl --shutdown</code> and changes networking for every distro,
        so CryptEnv never edits this file for you.
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
          {distro.defaultUser ?? 'unknown user'} ·{' '}
          <span className={distro.configured ? 'text-accent' : ''}>
            {distro.configured ? 'Configured' : 'Not configured'}
          </span>
        </div>
      </div>
      <button onClick={() => run('configure')} disabled={busy} className={BTN}>
        {configure.isPending ? 'CONFIGURING…' : distro.configured ? 'RECONFIGURE' : 'CONFIGURE'}
      </button>
      {distro.configured && (
        <button onClick={() => run('remove')} disabled={busy} className={BTN}>
          {remove.isPending ? 'REMOVING…' : 'REMOVE'}
        </button>
      )}
    </div>
  );
}

function ReportPanel() {
  const last = useWslStore((s) => s.lastAction);
  if (!last) return null;
  const { distro, action, report } = last;

  const lines: string[] = [];
  if (action === 'configure') {
    lines.push(`Wrote ${report.env_file}`);
    if (report.rc_files.length === 0) lines.push('Shell block already present — rc files unchanged');
    report.rc_files.forEach((rc) => lines.push(`Added the cryptenv block to ${rc}`));
    report.backups.forEach((b) => lines.push(`Backup written to ${b}`));
  } else {
    if (!report.env_file_changed && report.rc_files.length === 0) lines.push('Nothing to remove');
    report.rc_files.forEach((rc) => lines.push(`Removed the cryptenv block from ${rc}`));
    if (report.env_file_changed) lines.push(`Deleted ${report.env_file}`);
  }

  return (
    <div data-testid="wsl-report" className="mt-2 mb-2 px-3 py-2.5 rounded-[3px] border border-bd bg-raised">
      <div className="text-[12px] font-semibold text-tx font-ui mb-1">
        {action === 'configure' ? `Configured ${distro}` : `Removed from ${distro}`}
      </div>
      <ul className="text-[11px] font-mono text-tx3 leading-[1.8]">
        {lines.map((l) => <li key={l}>{l}</li>)}
      </ul>
      {action === 'configure' && (
        <div className="text-[11px] font-mono text-tx2 mt-1">Open a new WSL terminal to apply.</div>
      )}
    </div>
  );
}
