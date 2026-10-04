import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { WslActionReport, WslStatus } from '../../types';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { WslIntegrationSection, WSLCONFIG_SNIPPET, launcherLine } from './WslIntegrationSection';
import { useWslStore } from '../../store/wslStore';

const TWO_DISTROS: WslStatus = {
  available: true,
  mirrored:  false,
  distros: [
    { name: 'Ubuntu', state: 'running', defaultUser: 'me',   configured: true,  launcher: true  },
    { name: 'Debian', state: 'running', defaultUser: null,   configured: false, launcher: false },
  ],
};

const REPORT: WslActionReport = {
  env_file:         '/home/me/.config/cryptenv/env.sh',
  env_file_changed: true,
  rc_files:         ['/home/me/.bashrc'],
  backups:          ['/home/me/.bashrc.cryptenv.bak'],
  marker_added:     true,
  marker_removed:   false,
  launcher:         '/home/me/.local/share/cryptenv/bin/crypt-env',
  launcher_status:  'written',
  launcher_note:    null,
};

function mockCommands(detect: () => Promise<unknown>, action?: () => Promise<unknown>) {
  invoke.mockImplementation((cmd: string) => {
    if (cmd === 'wsl_detect') return detect();
    if (cmd === 'wsl_configure_client' || cmd === 'wsl_remove_client') {
      return action ? action() : Promise.resolve(REPORT);
    }
    return Promise.reject(new Error(`unexpected ${cmd}`));
  });
}

function renderSection(isWindows: boolean) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <WslIntegrationSection isWindows={isWindows} />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  invoke.mockReset();
  useWslStore.setState({ selectedDistro: null, lastAction: null });
});
afterEach(cleanup);

describe('WslIntegrationSection gating', () => {
  it('renders nothing and never invokes on a non-Windows platform', async () => {
    mockCommands(() => Promise.resolve(TWO_DISTROS));
    const { container } = renderSection(false);
    await new Promise((r) => setTimeout(r, 10));
    expect(container.innerHTML).toBe('');
    expect(invoke).not.toHaveBeenCalled();
  });

  it('renders nothing on Windows when no distro is detected', async () => {
    mockCommands(() => Promise.resolve({ available: false, distros: [], mirrored: false }));
    const { container } = renderSection(true);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('wsl_detect'));
    await new Promise((r) => setTimeout(r, 10));
    expect(container.innerHTML).toBe('');
  });

  it('lists distros with user and configured state on Windows with WSL', async () => {
    mockCommands(() => Promise.resolve(TWO_DISTROS));
    renderSection(true);
    const ubuntu = await screen.findByTestId('wsl-distro-Ubuntu');
    expect(ubuntu.textContent).toContain('me');
    expect(ubuntu.textContent).toContain('Configured');
    expect(ubuntu.textContent).toContain('REMOVE');
    const debian = screen.getByTestId('wsl-distro-Debian');
    expect(debian.textContent).toContain('Not configured');
    expect(debian.textContent).not.toContain('REMOVE');
  });

  it('flags a configured distro without the crypt-env command and offers reconfigure', async () => {
    mockCommands(() => Promise.resolve({
      ...TWO_DISTROS,
      distros: [{ name: 'Ubuntu', defaultUser: 'me', configured: true, launcher: false }],
    }));
    renderSection(true);
    const ubuntu = await screen.findByTestId('wsl-distro-Ubuntu');
    expect(ubuntu.textContent).toContain('no crypt-env command');
    expect(ubuntu.textContent).toContain('RECONFIGURE');
  });

  it('shows a non-fatal "could not detect" state on a tooling error', async () => {
    mockCommands(() => Promise.reject({ kind: 'tooling', message: 'unrecognised output from wsl.exe' }));
    renderSection(true);
    const note = await screen.findByRole('status');
    expect(note.textContent).toContain('Could not detect WSL');
    expect(note.textContent).toContain('unrecognised output');
  });
});

describe('mirrored networking banner', () => {
  it('shows the snippet and wsl --shutdown caveat when mirrored is off, with no apply control', async () => {
    mockCommands(() => Promise.resolve(TWO_DISTROS));
    renderSection(true);
    const banner = await screen.findByTestId('wsl-mirrored-banner');
    expect(banner.textContent).toContain(WSLCONFIG_SNIPPET.split('\n')[1]);
    expect(banner.textContent).toContain('wsl --shutdown');
    expect(banner.textContent).toContain('Not needed for the crypt-env command');
    expect(banner.textContent).toContain('native Linux crypt-env');
    const buttons = Array.from(banner.querySelectorAll('button'));
    expect(buttons).toHaveLength(1);
    expect(buttons[0].getAttribute('aria-label')).toBe('Copy .wslconfig snippet');
  });

  it('is hidden when mirrored networking is enabled', async () => {
    mockCommands(() => Promise.resolve({ ...TWO_DISTROS, mirrored: true }));
    renderSection(true);
    await screen.findByTestId('wsl-distro-Ubuntu');
    expect(screen.queryByTestId('wsl-mirrored-banner')).toBeNull();
  });
});

describe('configure / remove', () => {
  it('configures a distro, echoes the action report and re-runs detection', async () => {
    mockCommands(() => Promise.resolve(TWO_DISTROS));
    renderSection(true);
    const debian = await screen.findByTestId('wsl-distro-Debian');
    fireEvent.click(debian.querySelector('button') as HTMLButtonElement);

    const report = await screen.findByTestId('wsl-report');
    expect(invoke).toHaveBeenCalledWith('wsl_configure_client', { distro: 'Debian' });
    expect(report.textContent).toContain('Configured Debian');
    expect(report.textContent).toContain('/home/me/.bashrc.cryptenv.bak');
    expect(report.textContent).toContain('Added the cryptenv block to /home/me/.bashrc');
    expect(report.textContent).toContain('Installed the crypt-env command at /home/me/.local/share/cryptenv/bin/crypt-env');
    await waitFor(() =>
      expect(invoke.mock.calls.filter(([c]) => c === 'wsl_detect')).toHaveLength(2),
    );
  });

  it('removes from a configured distro and reports a no-op honestly', async () => {
    mockCommands(
      () => Promise.resolve(TWO_DISTROS),
      () => Promise.resolve({
        ...REPORT, env_file_changed: false, rc_files: [], backups: [], marker_added: false,
        launcher: null, launcher_status: 'absent',
      }),
    );
    renderSection(true);
    const ubuntu = await screen.findByTestId('wsl-distro-Ubuntu');
    const remove = Array.from(ubuntu.querySelectorAll('button')).find((b) => b.textContent === 'REMOVE');
    fireEvent.click(remove as HTMLButtonElement);

    const report = await screen.findByTestId('wsl-report');
    expect(invoke).toHaveBeenCalledWith('wsl_remove_client', { distro: 'Ubuntu' });
    expect(report.textContent).toContain('Removed from Ubuntu');
    expect(report.textContent).toContain('Nothing to remove');
  });

  it('reports a deleted launcher on remove instead of "Nothing to remove"', async () => {
    mockCommands(
      () => Promise.resolve(TWO_DISTROS),
      () => Promise.resolve({
        ...REPORT, env_file_changed: false, rc_files: [], backups: [], marker_added: false,
        launcher_status: 'deleted',
      }),
    );
    renderSection(true);
    const ubuntu = await screen.findByTestId('wsl-distro-Ubuntu');
    const remove = Array.from(ubuntu.querySelectorAll('button')).find((b) => b.textContent === 'REMOVE');
    fireEvent.click(remove as HTMLButtonElement);

    const report = await screen.findByTestId('wsl-report');
    expect(report.textContent).toContain('Deleted the crypt-env command');
    expect(report.textContent).not.toContain('Nothing to remove');
  });
});

describe('launcherLine', () => {
  it('describes every launcher outcome and tolerates reports from older helpers', () => {
    const base = { ...REPORT, launcher: '/l' };
    expect(launcherLine({ ...base, launcher_status: 'unchanged' })).toBe('crypt-env command already up to date at /l');
    expect(launcherLine({ ...base, launcher_status: 'skipped', launcher_note: 'windows CLI not found' }))
      .toBe('Skipped the crypt-env command (windows CLI not found)');
    expect(launcherLine({ ...base, launcher_status: 'absent' })).toBeNull();
    const { launcher: _l, launcher_status: _s, launcher_note: _n, ...legacy } = REPORT;
    expect(launcherLine(legacy)).toBeNull();
  });
});

describe('stopped distributions', () => {
  it('shows a stopped distro with a Detect action and only then probes it', async () => {
    const stopped: WslStatus = {
      ...TWO_DISTROS,
      distros: [
        TWO_DISTROS.distros[0],
        { name: 'Debian', state: 'stopped', defaultUser: null, configured: false, launcher: false },
      ],
    };
    const probed = { name: 'Debian', state: 'running', defaultUser: 'deb', configured: false, launcher: false };
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'wsl_detect') return Promise.resolve(stopped);
      if (cmd === 'wsl_detect_distro') return Promise.resolve(probed);
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    renderSection(true);
    const row = await screen.findByTestId('wsl-distro-Debian');
    expect(row.textContent).toContain('Stopped');
    expect(invoke).not.toHaveBeenCalledWith('wsl_detect_distro', expect.anything());

    fireEvent.click(screen.getByText('Detect (starts the distribution)'));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('wsl_detect_distro', { distro: 'Debian' }));
    await waitFor(() => expect(screen.getByTestId('wsl-distro-Debian').textContent).toContain('deb'));
  });
});
