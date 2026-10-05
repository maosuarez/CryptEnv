import { beforeEach, describe, expect, it, vi } from 'vitest';

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

import { useProjectStore } from './projectStore';

describe('createFromTemplates', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (cmd: string) => (cmd === 'project_list' ? [] : 7));
  });

  it('asks the backend for the baseline environments of a new project', async () => {
    await useProjectStore.getState().createFromTemplates({
      name: 'app', template: 'generic', categories: [], rootPath: '/work/app', vars: [],
    });
    const call = invokeMock.mock.calls.find(([c]) => c === 'project_create_from_templates');
    expect(call?.[1].input.project.seedBaseline).toBe(true);
    expect(call?.[1].input.project.initialEnvironment).toBeNull();
  });

  it('plain project saves (edits, imports) never ask for them', async () => {
    await useProjectStore.getState().saveProject({ name: 'app', template: 'generic', categories: [] });
    const call = invokeMock.mock.calls.find(([c]) => c === 'project_save');
    expect(call?.[1].project.seedBaseline).toBeUndefined();
  });
});
