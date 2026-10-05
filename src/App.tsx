import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useQueryClient } from '@tanstack/react-query';
import { AnimatePresence, motion } from 'framer-motion';
import { WindowChrome } from './components/WindowChrome';
import { LockScreen } from './components/LockScreen';
import { GlobalSecrets } from './components/GlobalSecrets';
import { EditItem } from './components/EditItem';
import { CategoryManager } from './components/CategoryManager';
import { Settings } from './components/Settings';
import { ProjectManager } from './components/ProjectManager';
import { ContextMenu } from './components/ui/ContextMenu';
import { Toast } from './components/ui/Toast';
import { PlaceholderModal } from './components/ui/PlaceholderModal';
import { SetupWizard } from './components/SetupWizard';
import { UpdateNotice } from './components/ui/UpdateNotice';
import { HotkeyNotice } from './components/ui/HotkeyNotice';
import { RecoveryScreen } from './components/RecoveryScreen';
import { ApprovalModal } from './components/ApprovalModal';
import { useVaultStore } from './store';
import { useUpdateStore } from './store/updateStore';
import { useAutoLock } from './hooks/useAutoLock';
import { debounce, isRefreshShortcut, manualRefresh, refreshVault, REFRESH_DEBOUNCE_MS } from './lib/vaultRefresh';
import type { Screen } from './types';

const SCREENS: Record<Screen, React.ReactElement> = {
  lock:       <LockScreen />,
  vault:      <GlobalSecrets />,
  edit:       <EditItem />,
  categories: <CategoryManager />,
  settings:   <Settings />,
  projects:   <ProjectManager />,
};

/** Startup gate: if the vault database could not be opened the backend runs
 *  in recovery mode and only the recovery screen is usable. */
export default function App() {
  const [mode, setMode] = useState<{ mode: string; error: string | null } | null>(null);

  useEffect(() => {
    invoke<{ mode: string; error: string | null }>('app_mode')
      .then(setMode)
      .catch(() => setMode({ mode: 'normal', error: null }));
  }, []);

  if (mode === null) return <div className="w-full h-full bg-bg" />;
  if (mode.mode === 'recovery') return <RecoveryScreen error={mode.error ?? ''} />;
  return <MainApp />;
}

function MainApp() {
  useAutoLock();
  const screen         = useVaultStore((s) => s.screen);
  const menu           = useVaultStore((s) => s.menu);
  const closeMenu      = useVaultStore((s) => s.closeMenu);
  const toast          = useVaultStore((s) => s.toast);
  const placeholder    = useVaultStore((s) => s.placeholder);
  const setPlaceholder = useVaultStore((s) => s.setPlaceholder);
  const checkForUpdate = useUpdateStore((s) => s.checkOnce);

  const lockedByBackend = useVaultStore((s) => s.lockedByBackend);

  // Backend auto-lock already zeroized the key — mirror it immediately.
  useEffect(() => {
    const unlisten = listen('vault_locked', () => lockedByBackend());
    return () => { unlisten.then((f) => f()).catch(() => {}); };
  }, [lockedByBackend]);

  // Another client (CLI, TUI, MCP) changed the vault: re-read what is shown.
  // The event has no payload; a burst of events becomes one refetch.
  const queryClient = useQueryClient();
  useEffect(() => {
    const refresh = debounce(() => { void refreshVault(queryClient); }, REFRESH_DEBOUNCE_MS);
    const unlisten = listen('vault_changed', () => refresh.call());
    return () => {
      refresh.cancel();
      unlisten.then((f) => f()).catch(() => {});
    };
  }, [queryClient]);

  // F5 / Ctrl+R / Cmd+R refresh the vault data instead of reloading the webview.
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (!isRefreshShortcut(e)) return;
      e.preventDefault();
      void manualRefresh(queryClient);
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [queryClient]);

  const showSetupWizard = useVaultStore((s) => s.wizardOpen);
  const setWizardOpen   = useVaultStore((s) => s.setWizardOpen);
  const prevScreenRef = useRef<Screen>(screen);

  useEffect(() => {
    const prev = prevScreenRef.current;
    prevScreenRef.current = screen;
    if (prev === 'lock' && screen === 'projects') {
      invoke<boolean>('app_is_first_run')
        .then((isFirst) => { if (isFirst) setWizardOpen(true); })
        .catch(() => {});
    }
    // First unlock of this launch → background update check (no-op afterwards).
    if (prev === 'lock' && screen !== 'lock') checkForUpdate();
  }, [screen, checkForUpdate, setWizardOpen]);

  return (
    <div className="flex flex-col w-full h-full bg-bg overflow-hidden">
      <WindowChrome />

      {/* Screen area */}
      <div className="flex-1 flex flex-col overflow-hidden relative">
        <AnimatePresence mode="wait">
          <motion.div
            key={screen}
            className="absolute inset-0 flex flex-col overflow-hidden"
            initial={{ opacity: 0, y: 4 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -4 }}
            transition={{ duration: 0.13, ease: 'easeOut' }}
          >
            {SCREENS[screen]}
          </motion.div>
        </AnimatePresence>
      </div>

      {/* Global overlays */}
      <UpdateNotice />
      <HotkeyNotice />
      <ApprovalModal />
      {menu && <ContextMenu {...menu} onClose={closeMenu} />}
      {toast && <Toast msg={toast.type === 'error' ? toast.msg : `✓ ${toast.msg}`} type={toast.type} />}
      {/* Secret-bearing overlays never render over the lock screen. */}
      {screen !== 'lock' && placeholder && placeholder.type === 'command' && (
        <PlaceholderModal
          command={(placeholder as any).command}
          onClose={() => setPlaceholder(null)}
        />
      )}
      {screen !== 'lock' && showSetupWizard && (
        <SetupWizard onClose={() => setWizardOpen(false)} />
      )}
    </div>
  );
}
