import { useVaultStore } from '../../store';
import { useUpdateStore } from '../../store/updateStore';

const BTN = 'h-7 px-3 rounded-[3px] text-[11px] font-semibold tracking-[0.06em] font-ui cursor-pointer transition-colors disabled:opacity-40 disabled:cursor-default';

export function UpdateNotice() {
  const screen     = useVaultStore((s) => s.screen);
  const version    = useUpdateStore((s) => s.version);
  const dismissed  = useUpdateStore((s) => s.dismissed);
  const installing = useUpdateStore((s) => s.installing);
  const installed  = useUpdateStore((s) => s.installed);
  const error      = useUpdateStore((s) => s.error);
  const install    = useUpdateStore((s) => s.install);
  const dismiss    = useUpdateStore((s) => s.dismiss);

  if (!version || dismissed || screen === 'lock') return null;

  return (
    <div
      role="status"
      className="fixed bottom-14 right-4 z-[8000] w-[280px] bg-raised border border-accent-d rounded-[4px] px-[14px] py-[10px] shadow-[0_4px_20px_rgba(0,0,0,.6)] animate-fade-in"
    >
      <div className="text-[12px] font-mono text-accent">Update available: v{version}</div>
      {installed ? (
        <div className="mt-1 text-[12px] font-mono text-tx2">Installed — restart the app to apply</div>
      ) : (
        <>
          {error && <div className="mt-1 text-[12px] font-mono text-danger break-words">{error}</div>}
          <div className="mt-2 flex justify-end gap-2">
            <button
              onClick={dismiss}
              disabled={installing}
              className={`${BTN} bg-transparent border border-bd2 text-tx2 hover:text-tx`}
            >
              LATER
            </button>
            <button
              onClick={install}
              disabled={installing}
              className={`${BTN} bg-accent-b border border-accent-d text-accent hover:text-tx`}
            >
              {installing ? 'INSTALLING…' : 'INSTALL'}
            </button>
          </div>
        </>
      )}
    </div>
  );
}
