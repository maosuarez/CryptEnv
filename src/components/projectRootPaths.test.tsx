import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/plugin-os', () => ({ platform: () => 'windows' }));

import { InjectTargetModal, relativeToRoot } from './ProjectManager';

describe('relativeToRoot (env paths picked inside the project root)', () => {
  it('stores paths inside the root relative with forward slashes', () => {
    expect(relativeToRoot('C:\\work\\app', 'C:\\work\\app\\apps\\api\\.env')).toBe('apps/api/.env');
    expect(relativeToRoot('\\\\wsl.localhost\\Ubuntu\\home\\u\\app\\', '\\\\wsl.localhost\\Ubuntu\\home\\u\\app\\.env')).toBe('.env');
    expect(relativeToRoot('/home/u/app', '/home/u/app/web/.env.local')).toBe('web/.env.local');
  });

  it('keeps paths outside the root (or without a root) unchanged', () => {
    expect(relativeToRoot('/home/u/app', '/home/u/application/.env')).toBe('/home/u/application/.env');
    expect(relativeToRoot(undefined, 'C:\\x\\.env')).toBe('C:\\x\\.env');
  });
});

describe('InjectTargetModal', () => {
  afterEach(cleanup);
  const PATHS = ['apps/web/.env', 'apps/api/.env'];

  it('confirms all paths as null (inject everywhere) by default', () => {
    const onConfirm = vi.fn();
    render(<InjectTargetModal paths={PATHS} onCancel={() => {}} onConfirm={onConfirm} />);
    fireEvent.click(screen.getByText('INJECT'));
    expect(onConfirm).toHaveBeenCalledWith(null);
  });

  it('confirms only the checked subset', () => {
    const onConfirm = vi.fn();
    render(<InjectTargetModal paths={PATHS} onCancel={() => {}} onConfirm={onConfirm} />);
    fireEvent.click(screen.getByText('apps/web/.env'));
    fireEvent.click(screen.getByText('INJECT'));
    expect(onConfirm).toHaveBeenCalledWith(['apps/api/.env']);
  });

  it('disables inject when nothing is selected', () => {
    const onConfirm = vi.fn();
    render(<InjectTargetModal paths={PATHS} onCancel={() => {}} onConfirm={onConfirm} />);
    fireEvent.click(screen.getByText('All configured paths'));
    expect((screen.getByText('INJECT') as HTMLButtonElement).disabled).toBe(true);
  });
});
