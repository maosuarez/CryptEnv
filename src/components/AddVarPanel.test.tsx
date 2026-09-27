import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { Project } from '../types';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock('@tauri-apps/plugin-os', () => ({ platform: () => 'windows' }));

import { AddVarPanel } from './ProjectManager';
import { useVaultStore } from '../store';

const PROJECT = { id: 7, name: 'api', environments: [], categories: [] } as unknown as Project;
const GLOBALS = [{ id: 42, name: 'DATABASE_URL' }, { id: 43, name: 'OPENAI_API_KEY' }];

function renderPanel(onAdded = vi.fn()) {
  render(<AddVarPanel project={PROJECT} globalItems={GLOBALS} onAdded={onAdded} onCancel={() => {}} />);
  fireEvent.click(screen.getByText('NEW ITEM'));
  return onAdded;
}

const keyInput   = () => screen.getByPlaceholderText('KEY_NAME — e.g. DATABASE_URL') as HTMLInputElement;
const vaultInput = () => screen.getByPlaceholderText('e.g. OpenAI — production') as HTMLInputElement;

describe('AddVarPanel (new variable)', () => {
  beforeEach(() => { invoke.mockReset(); });
  afterEach(cleanup);

  it('mirrors the KEY into the vault name until the name is edited', () => {
    renderPanel();
    fireEvent.change(keyInput(), { target: { value: 'STRIPE_KEY' } });
    expect(vaultInput().value).toBe('STRIPE_KEY');
    fireEvent.change(vaultInput(), { target: { value: 'Stripe prod' } });
    fireEvent.change(keyInput(), { target: { value: 'STRIPE_SECRET' } });
    expect(vaultInput().value).toBe('Stripe prod');
  });

  it('links a matching global secret instead of creating an item', () => {
    const onAdded = renderPanel();
    fireEvent.change(keyInput(), { target: { value: 'database' } });
    fireEvent.click(screen.getByRole('button', { name: /DATABASE_URL/ }));
    expect(screen.queryByPlaceholderText('sk-…')).toBeNull();
    fireEvent.click(screen.getByText('ADD'));
    expect(onAdded).toHaveBeenCalledWith(expect.objectContaining({ key: 'DATABASE', itemId: 42 }));
    expect(invoke).not.toHaveBeenCalled();
  });

  it('refreshes the vault store before linking a newly created item', async () => {
    const created = { id: 99, type: 'secret', name: 'NEW_KEY', value: 'v', created: '', isGlobal: false };
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'vault_create_project_item') return Promise.resolve(created);
      if (cmd === 'vault_list') return Promise.resolve({ items: [created], categories: [] });
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    const onAdded = vi.fn(() => {
      expect(useVaultStore.getState().items.some((i) => i.id === 99)).toBe(true);
    });
    renderPanel(onAdded);
    fireEvent.change(keyInput(), { target: { value: 'NEW_KEY' } });
    fireEvent.change(screen.getByPlaceholderText('sk-…'), { target: { value: 'v' } });
    fireEvent.click(screen.getByText('ADD'));
    await waitFor(() => expect(onAdded).toHaveBeenCalledWith(expect.objectContaining({ key: 'NEW_KEY', itemId: 99 })));
  });
});
