import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import type { Project, EnvironmentVar, InjectResult, InjectPreview, ProjectDeleteImpact, VaultItem } from '../types';

export interface EnvironmentInput {
  id?:        number;
  projectId:  number;
  name:       string;
  isDefault:  boolean;
  paths:      string[];
  vars:       EnvironmentVar[];
}

export interface ProjectFromTemplatesInput {
  name:         string;
  description?: string;
  template:     string;
  categories:   string[];
  /** Name of the auto-created environment; blank → "default". */
  initialEnvironment?: string;
  /** Project root directory (absolute, host view). */
  rootPath?:   string;
  vars:         Array<{ key: string; value: string }>;
}

interface ProjectStore {
  projects: Project[];
  loading:  boolean;
  error:    string | null;

  load:              () => Promise<void>;
  /** `rootPath`: omitted keeps the current root, `''` clears it. */
  saveProject:       (input: { id?: number; name: string; description?: string; template: string; categories: string[]; rootPath?: string }) => Promise<number>;
  createFromTemplates: (input: ProjectFromTemplatesInput) => Promise<number>;
  removeProject:     (id: number) => Promise<ProjectDeleteImpact>;
  previewDelete:     (id: number) => Promise<ProjectDeleteImpact>;
  saveEnvironment:   (input: EnvironmentInput) => Promise<number>;
  removeEnvironment: (id: number) => Promise<void>;
  /** `targets`: subset of the environment's configured paths; omitted = all. */
  inject:            (environmentId: number, overwrite?: boolean, targets?: string[]) => Promise<InjectResult>;
  previewInject:     (environmentId: number, targets?: string[]) => Promise<InjectPreview>;
  createProjectItem: (projectId: number, item: Omit<VaultItem, 'id' | 'created'>) => Promise<VaultItem>;
  clearError:        () => void;
}

export const useProjectStore = create<ProjectStore>((set, get) => ({
  projects: [],
  loading:  false,
  error:    null,

  load: async () => {
    set({ loading: true, error: null });
    try {
      const projects = await invoke<Project[]>('project_list');
      set({ projects, loading: false });
    } catch (e) {
      set({ loading: false, error: String(e) });
    }
  },

  saveProject: async (input) => {
    const id = await invoke<number>('project_save', {
      project: {
        id:          input.id ?? 0,
        name:        input.name,
        description: input.description ?? null,
        template:    input.template,
        categories:  input.categories,
        rootPath:    input.rootPath ?? null,
      },
    });
    await get().load();
    return id;
  },

  createFromTemplates: async (input) => {
    const id = await invoke<number>('project_create_from_templates', {
      input: {
        project: {
          id:          0,
          name:        input.name,
          description: input.description ?? null,
          template:    input.template,
          categories:  input.categories,
          initialEnvironment: input.initialEnvironment ?? null,
          rootPath:    input.rootPath ?? null,
          // New projects start with default + staging + production
          // (ignored by the backend when an initial environment is named).
          seedBaseline: true,
        },
        vars: input.vars,
      },
    });
    await get().load();
    return id;
  },

  removeProject: async (id) => {
    const impact = await invoke<ProjectDeleteImpact>('project_delete', { id });
    set((s) => ({ projects: s.projects.filter((p) => p.id !== id) }));
    return impact;
  },

  previewDelete: (id) => invoke<ProjectDeleteImpact>('project_preview_delete', { id }),

  saveEnvironment: async (input) => {
    const id = await invoke<number>('environment_save', {
      environment: {
        id:         input.id ?? 0,
        projectId:  input.projectId,
        name:       input.name,
        isDefault:  input.isDefault,
        paths:      input.paths,
        vars:       input.vars,
      },
    });
    await get().load();
    return id;
  },

  removeEnvironment: async (id) => {
    await invoke('environment_delete', { id });
    await get().load();
  },

  inject: async (environmentId, overwrite = false, targets) => {
    return invoke<InjectResult>('environment_inject', { id: environmentId, overwrite, targets: targets ?? null });
  },

  previewInject: (environmentId, targets) => {
    return invoke<InjectPreview>('environment_inject_preview', { id: environmentId, targets: targets ?? null });
  },

  createProjectItem: (projectId, item) => {
    return invoke<VaultItem>('vault_create_project_item', { item, projectId });
  },

  clearError: () => set({ error: null }),
}));
