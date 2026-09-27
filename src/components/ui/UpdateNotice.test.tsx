import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { UpdateNotice } from './UpdateNotice';
import { useUpdateStore } from '../../store/updateStore';
import { useVaultStore } from '../../store';

const INITIAL = useUpdateStore.getState();

beforeEach(() => {
  invoke.mockReset();
  useUpdateStore.setState(INITIAL, true);
  useVaultStore.setState({ screen: 'projects' });
});
afterEach(cleanup);

describe('updateStore.checkOnce', () => {
  it('checks only once per launch', async () => {
    invoke.mockResolvedValue(null);
    await useUpdateStore.getState().checkOnce();
    await useUpdateStore.getState().checkOnce();
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith('check_for_update');
  });

  it('stays silent when the check fails', async () => {
    invoke.mockRejectedValue(new Error('offline'));
    await useUpdateStore.getState().checkOnce();
    const s = useUpdateStore.getState();
    expect(s.version).toBeNull();
    expect(s.error).toBeNull();
    render(<UpdateNotice />);
    expect(screen.queryByRole('status')).toBeNull();
  });
});

describe('UpdateNotice', () => {
  it('renders nothing when no update is available', async () => {
    invoke.mockResolvedValue(null);
    await useUpdateStore.getState().checkOnce();
    render(<UpdateNotice />);
    expect(screen.queryByRole('status')).toBeNull();
  });

  it('shows the available version', async () => {
    invoke.mockResolvedValue('1.0.3');
    await useUpdateStore.getState().checkOnce();
    render(<UpdateNotice />);
    expect(screen.getByText('Update available: v1.0.3')).toBeTruthy();
  });

  it('is hidden on the lock screen and returns after unlock', () => {
    useUpdateStore.setState({ version: '1.0.3' });
    useVaultStore.setState({ screen: 'lock' });
    render(<UpdateNotice />);
    expect(screen.queryByRole('status')).toBeNull();
    act(() => useVaultStore.setState({ screen: 'projects' }));
    expect(screen.getByRole('status')).toBeTruthy();
  });

  it('LATER hides it for the rest of the launch', () => {
    useUpdateStore.setState({ version: '1.0.3' });
    render(<UpdateNotice />);
    fireEvent.click(screen.getByText('LATER'));
    expect(screen.queryByRole('status')).toBeNull();
  });

  it('INSTALL calls install_update and reports success', async () => {
    useUpdateStore.setState({ version: '1.0.3' });
    invoke.mockResolvedValue(undefined);
    render(<UpdateNotice />);
    fireEvent.click(screen.getByText('INSTALL'));
    expect(invoke).toHaveBeenCalledWith('install_update');
    await waitFor(() => expect(screen.getByText(/restart the app to apply/)).toBeTruthy());
  });

  it('INSTALL failure shows the error and re-enables the button', async () => {
    useUpdateStore.setState({ version: '1.0.3' });
    invoke.mockRejectedValue('signature mismatch');
    render(<UpdateNotice />);
    fireEvent.click(screen.getByText('INSTALL'));
    await waitFor(() => expect(screen.getByText('signature mismatch')).toBeTruthy());
    expect((screen.getByText('INSTALL') as HTMLButtonElement).disabled).toBe(false);
  });
});
