export type ItemType = 'secret' | 'credential' | 'link' | 'command' | 'note';
export type Shell    = 'bash' | 'zsh' | 'fish' | 'PowerShell' | 'cmd';
export type Screen   = 'lock' | 'vault' | 'edit' | 'categories' | 'settings' | 'projects';

export interface Category {
  id:    string;
  name:  string;
  color: string;
}

interface BaseItem {
  id:         number;
  type:       ItemType;
  categories: string[];
  notes?:     string;
  created:    string;
  /** Reusable across projects/environments — only global items are offered
   *  as vault-item references when linking an environment variable. */
  isGlobal?:  boolean;
}

export interface SecretItem extends BaseItem {
  type:  'secret';
  name:  string;
  value: string;
}

export interface CredentialItem extends BaseItem {
  type:     'credential';
  name:     string;
  url?:     string;
  username: string;
  password: string;
}

export interface LinkItem extends BaseItem {
  type:         'link';
  title:        string;
  url:          string;
  description?: string;
}

export interface CommandItem extends BaseItem {
  type:         'command';
  name:         string;
  command:      string;
  description?: string;
  shell:        Shell;
}

export interface NoteItem extends BaseItem {
  type:    'note';
  title:   string;
  content: string;
}

export type VaultItem = SecretItem | CredentialItem | LinkItem | CommandItem | NoteItem;

// ─── Project / Environment types ───────────────────────────────────────────────

export interface EnvironmentVar {
  id:     number;
  key:    string;
  itemId: number;
}

export interface Environment {
  id:        number;
  projectId: number;
  name:      string;
  isDefault: boolean;
  paths:     string[];
  vars:      EnvironmentVar[];
  created:   string;
  updated:   string;
}

export interface Project {
  id:           number;
  name:         string;
  description?: string;
  template:     string;
  created:      string;
  updated:      string;
  environments: Environment[];
  /** Category names — same convention as VaultItem.categories. Language is
   *  just another tag value here (e.g. "Python"). */
  categories:   string[];
  /** Project root directory as the vault host sees it (holds
   *  `.crypt-env.yaml`); relative environment paths resolve against it. */
  rootPath?:    string;
}

/** Comma-joined template ids from `src/data/projectTemplates.ts`, or
 *  `generic` when none was selected. Legacy projects hold a single id. */
export type ProjectTemplate = string;

export interface InjectResult {
  paths:          string[];
  written:        string[];
  /** Owner-configured paths that were unmanaged (pre-existing, not created
   *  by crypt-env) at the time of this inject — written anyway (configured
   *  paths are never hard-gated), but surfaced so the caller can see it.
   *  Self-heals: a path drops off this list on the next inject once it
   *  carries the marker. */
  unmanagedPaths: string[];
  /** `.bak` paths created because a write target was unmanaged. */
  backups:        string[];
}

/** Result of `environment_inject_preview` — resolves and inspects the
 *  environment's configured paths without decrypting or writing anything,
 *  so the GUI can show a confirm dialog before an inject that would
 *  overwrite unmanaged files. */
export interface InjectPreview {
  paths:   string[];
  foreign: string[];
}

export interface ProjectDeleteImpact {
  environments:   number;
  itemsDeleted:   number;
  itemsOrphaned:  number;
}

export interface ItemOwner {
  projectId:   number;
  projectName: string;
}

export interface GlobalToggleResult {
  updated: VaultItem | null;
  forked:  VaultItem[];
}

export interface ContextMenuItemDef {
  label?:   string;
  icon?:    string;
  onClick?: () => void;
  danger?:  boolean;
  divider?: boolean;
  sub?:     string;
}

export interface MenuState {
  x:     number;
  y:     number;
  items: ContextMenuItemDef[];
}

export type IconName =
  | 'lock'        | 'unlock'   | 'eye'      | 'eyeOff'  | 'copy'    | 'check'
  | 'plus'        | 'search'   | 'settings' | 'trash'   | 'edit'    | 'close'
  | 'back'        | 'shield'   | 'key'      | 'kbd'     | 'timer'   | 'person'
  | 'globe'       | 'terminal' | 'more'     | 'tag'     | 'drag'    | 'external'
  | 'export'      | 'rename'   | 'note'     | 'fingerprint' | 'refresh' | 'funnel';

// ─── WSL Integration (Settings, Windows only) ────────────────────────────────

export interface WslDistro {
  name:        string;
  defaultUser: string | null;
  configured:  boolean;
  /** The managed `crypt-env` launcher (delegating to the Windows CLI) is installed. */
  launcher:    boolean;
}

export interface WslStatus {
  available: boolean;
  distros:   WslDistro[];
  mirrored:  boolean;
}

/** Mirrors `cryptenv_setup::ActionReport` (snake_case — shared with the WSL helper's JSON). */
export interface WslActionReport {
  env_file:         string;
  env_file_changed: boolean;
  rc_files:         string[];
  backups:          string[];
  marker_added:     boolean;
  marker_removed:   boolean;
  launcher?:        string | null;
  launcher_status?: WslLauncherStatus;
  launcher_note?:   string | null;
}

export type WslLauncherStatus = 'absent' | 'written' | 'unchanged' | 'deleted' | 'skipped';

export interface WslError {
  kind:     'unsupported' | 'notAvailable' | 'unknownDistro' | 'tooling';
  message?: string;
}
