import { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { platform } from '@tauri-apps/plugin-os';
import { Icon } from './ui/Icon';
import { TagInput } from './ui/TagInput';
import { ProjectShareModal, type ProjectShareMode } from './ProjectShareModal';
import { useVaultStore, isVaultLockedError } from '../store';
import { useProjectStore } from '../store/projectStore';
import { useTranslation } from '../i18n';
import {
  ItemTypePicker, ItemTypeFields, emptyItemFields, validateItemFields, itemNameKey, Label, F,
  type ItemFieldsState,
} from './itemFields/ItemTypeFields';
import type {
  EnvironmentVar, Environment, Project, ProjectTemplate, ProjectDeleteImpact, InjectResult, VaultItem, ItemType, Category,
} from '../types';
import {
  TEMPLATE_GROUPS, TEMPLATE_PLACEHOLDER, getTemplate, isValidEnvKey, mergeTemplateVars, normalizeEnvKey,
  searchTemplates, templateCategories, templateLabels, templateString, type MergedVar,
} from '../data/projectTemplates';
import { CAT_COLORS_PRESET } from '../store';

interface ExportedEnvironment {
  name:      string;
  isDefault: boolean;
  vars:      Array<{ key: string; literal?: string }>;
}

interface ExportedProject {
  version:      number;
  name:         string;
  description?: string;
  template:     string;
  environments: ExportedEnvironment[];
}

/** Routes "vault is locked" to the lock screen (the session is gone, so a
 *  toast alone would leave a dead form); everything else to an error toast. */
function reportError(e: unknown) {
  const s = useVaultStore.getState();
  if (isVaultLockedError(e)) s.lockedByBackend();
  else s.showToast(String(e), 'error');
}

// Values stay lowercase: an environment name becomes the `.env.<name>`
// filename on inject, and frameworks (Next.js, Vite, dotenv) look for
// `.env.production` on case-sensitive filesystems. Only the displayed label
// is capitalized and localized (it may contain non-ASCII, which env names
// can't).
const ENV_PRESETS = ['production', 'local', 'test', 'staging'] as const;

function EnvPresetOptions({ id }: { id: string }) {
  const { t } = useTranslation();
  return (
    <datalist id={id}>
      {ENV_PRESETS.map((n) => <option key={n} value={n} label={t(`projects.envPresets.${n}`)} />)}
    </datalist>
  );
}

// ─── Name validation (UX mirror only — see issue #7) ──────────────────────────
//
// These mirror `validate_environment_name` / `validate_project_name` in
// `src-tauri/src/project/mod.rs` purely to avoid a pointless round-trip to
// the server. The server is the actual enforcement point (reachable from
// the GUI, HTTP API, CLI, and an imported `.cryptenv-proj` template) — this
// copy may drift from it over time and that is an accepted risk, not a bug.

function isValidEnvironmentName(name: string): boolean {
  return /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/.test(name) && !name.endsWith('.') && !name.endsWith('-');
}

function isValidProjectName(name: string): boolean {
  if (name.trim().length === 0 || name.length > 128) return false;
  if (name === '.' || name === '..') return false;
  // eslint-disable-next-line no-control-regex
  if (/[\x00-\x1f/\\:<>"|?*]/.test(name)) return false;
  if (name.endsWith('.') || name.endsWith(' ') || name.startsWith('.') || name.startsWith(' ')) return false;
  return true;
}

function TemplatePicker({
  initial,
  onConfirm,
  onClose,
}: {
  initial:   string[];
  onConfirm: (ids: string[]) => void;
  onClose:   () => void;
}) {
  const { t } = useTranslation();
  const [query, setQuery]       = useState('');
  const [selected, setSelected] = useState<string[]>(initial);

  const visible = useMemo(() => searchTemplates(query), [query]);
  const toggle = (id: string) =>
    setSelected((cur) => (cur.includes(id) ? cur.filter((x) => x !== id) : [...cur, id]));

  return (
    <div
      className="absolute inset-0 bg-black/70 flex items-center justify-center z-30 p-4"
      onKeyDown={(e) => { if (e.key === 'Escape') onClose(); }}
    >
      <div className="w-full max-w-2xl max-h-full flex flex-col bg-surface border border-bd rounded-[4px] p-4">
        <div className="flex items-center gap-2 mb-3">
          <div className="flex-1 text-[10px] font-semibold text-tx3 font-mono tracking-[0.09em]">
            {t('projects.templateModal.header')}
          </div>
          <span className="text-[10px] font-mono text-accent">
            {t('projects.templateModal.selectedCount', { n: selected.length })}
          </span>
          {selected.length > 0 && (
            <button
              onClick={() => setSelected([])}
              className="text-[10px] font-ui font-medium text-tx2 border border-bd2 rounded-[3px] px-2 py-[2px] hover:text-tx transition-colors"
            >
              {t('projects.templateModal.clear')}
            </button>
          )}
        </div>
        <input
          autoFocus
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t('projects.templateModal.searchPlaceholder')}
          className="w-full bg-bg border border-bd2 text-tx font-mono text-[12px] rounded-[3px] px-3 py-[7px] mb-3 outline-none focus:border-accent-d transition-colors"
        />
        <div className="flex-1 min-h-0 h-[360px] overflow-y-auto pr-1">
          {visible.length === 0 && (
            <div className="text-[11px] text-tx3 font-mono py-6 text-center">{t('projects.templateModal.noResults')}</div>
          )}
          {TEMPLATE_GROUPS.map((g) => {
            const inGroup = visible.filter((tpl) => tpl.group === g);
            if (inGroup.length === 0) return null;
            return (
              <div key={g} className="mb-3">
                <div className="text-[9px] font-semibold text-tx3 font-mono tracking-[0.12em] mb-1.5 pb-1 border-b border-bd uppercase">
                  {t(`projects.templateModal.groups.${g}`)}
                </div>
                <div className="grid grid-cols-2 sm:grid-cols-3 gap-1.5">
                  {inGroup.map((tpl) => {
                    const on = selected.includes(tpl.id);
                    return (
                      <button
                        key={tpl.id}
                        onClick={() => toggle(tpl.id)}
                        aria-pressed={on}
                        className={[
                          'px-2.5 py-2 border rounded-[3px] text-left transition-colors',
                          on ? 'bg-accent-b border-accent-d' : 'bg-raised border-bd2 hover:border-accent-d',
                        ].join(' ')}
                      >
                        <div className="flex items-center gap-1.5">
                          <span className={`text-[12px] font-semibold font-ui truncate flex-1 ${on ? 'text-accent' : 'text-tx'}`}>
                            {tpl.label}
                          </span>
                          {on && <span className="text-accent"><Icon name="check" size={11} /></span>}
                        </div>
                        <div className="text-[10px] text-tx3 font-mono mt-0.5">
                          {t('projects.templateModal.varsCount', { n: tpl.vars.length })}
                        </div>
                      </button>
                    );
                  })}
                </div>
              </div>
            );
          })}
        </div>
        <div className="flex gap-2 mt-3">
          <button
            onClick={onClose}
            className="flex-1 py-[7px] rounded-[3px] text-[11px] font-bold tracking-[0.06em] font-ui cursor-pointer bg-transparent border border-bd2 text-tx2 hover:text-tx transition-colors"
          >
            {t('common.cancel')}
          </button>
          <button
            onClick={() => onConfirm(selected)}
            className="flex-1 py-[7px] rounded-[3px] text-[11px] font-bold tracking-[0.06em] font-ui cursor-pointer bg-accent border-none text-[#020504] hover:opacity-90 transition-opacity"
          >
            {selected.length === 0 ? t('projects.templateModal.continueEmpty') : t('projects.templateModal.continue')}
          </button>
        </div>
      </div>
    </div>
  );
}

// ─── TemplateVarsReview (editable merged template variables) ──────────────────

interface ReviewVar extends MergedVar {
  uid: number;
}

let reviewUid = 0;
const toReviewVars = (merged: MergedVar[]): ReviewVar[] =>
  merged.map((v) => ({ ...v, alsoIn: [...v.alsoIn], uid: ++reviewUid }));

/** Keys that are empty or collide — used to block creation. */
function invalidReviewKeys(vars: ReviewVar[]): Set<number> {
  const bad = new Set<number>();
  const seen = new Map<string, number>();
  for (const v of vars) {
    if (!isValidEnvKey(v.key)) bad.add(v.uid);
    const prev = seen.get(v.key);
    if (prev !== undefined) { bad.add(prev); bad.add(v.uid); } else seen.set(v.key, v.uid);
  }
  return bad;
}

function TemplateVarsReview({
  vars,
  onChange,
}: {
  vars:     ReviewVar[];
  onChange: (vars: ReviewVar[]) => void;
}) {
  const { t } = useTranslation();
  const [revealed, setRevealed] = useState<Set<number>>(new Set());
  const invalid = useMemo(() => invalidReviewKeys(vars), [vars]);

  const update = (uid: number, patch: Partial<ReviewVar>) =>
    onChange(vars.map((v) => (v.uid === uid ? { ...v, ...patch } : v)));
  const remove = (uid: number) => onChange(vars.filter((v) => v.uid !== uid));
  const add = () =>
    onChange([...vars, { key: '', value: '', sensitive: false, source: '', alsoIn: [], uid: ++reviewUid }]);
  const toggleReveal = (uid: number) =>
    setRevealed((r) => { const n = new Set(r); if (n.has(uid)) n.delete(uid); else n.add(uid); return n; });

  // Group rows by source template, preserving first-appearance order.
  const groups: { source: string; rows: ReviewVar[] }[] = [];
  for (const v of vars) {
    const g = groups.find((x) => x.source === v.source);
    if (g) g.rows.push(v); else groups.push({ source: v.source, rows: [v] });
  }

  return (
    <div className="mb-3">
      <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.06em] mb-1 flex items-center justify-between">
        <span>{t('projects.review.header', { n: vars.length })}</span>
        <button
          onClick={add}
          className="flex items-center gap-1 text-accent text-[10px] font-ui font-bold hover:opacity-80 transition-opacity"
        >
          <Icon name="plus" size={10} />{t('projects.review.add')}
        </button>
      </div>
      {vars.length === 0 && (
        <div className="text-[11px] text-tx3 font-mono py-2">{t('projects.review.empty')}</div>
      )}
      {groups.map((g) => (
        <div key={g.source || '_custom'} className="mb-2">
          <div className="text-[9px] font-mono text-tx3 tracking-[0.1em] uppercase mb-1">
            {g.source ? (getTemplate(g.source)?.label ?? g.source) : t('projects.review.custom')}
          </div>
          {g.rows.map((v) => {
            const bad = invalid.has(v.uid);
            const masked = v.sensitive && !revealed.has(v.uid);
            return (
              <div key={v.uid} className="mb-1">
                <div className="flex items-center gap-1.5">
                  <input
                    value={v.key}
                    onChange={(e) => update(v.uid, { key: normalizeEnvKey(e.target.value) })}
                    placeholder="KEY"
                    aria-invalid={bad}
                    className={`w-[42%] bg-bg border text-tx font-mono text-[11px] rounded-[3px] px-2 py-[5px] outline-none transition-colors ${bad ? 'border-danger' : 'border-bd2 focus:border-accent-d'}`}
                  />
                  <input
                    type={masked ? 'password' : 'text'}
                    value={v.value}
                    onChange={(e) => update(v.uid, { value: e.target.value })}
                    placeholder={TEMPLATE_PLACEHOLDER}
                    className="flex-1 min-w-0 bg-bg border border-bd2 text-tx font-mono text-[11px] rounded-[3px] px-2 py-[5px] outline-none focus:border-accent-d transition-colors"
                  />
                  {v.sensitive && (
                    <button
                      onClick={() => toggleReveal(v.uid)}
                      aria-label={masked ? t('projects.review.reveal') : t('projects.review.hide')}
                      className="text-tx3 hover:text-tx transition-colors"
                    >
                      <Icon name={masked ? 'eye' : 'eyeOff'} size={12} />
                    </button>
                  )}
                  <button
                    onClick={() => remove(v.uid)}
                    aria-label={t('projects.review.remove')}
                    className="text-tx3 hover:text-danger transition-colors text-[14px] leading-none px-0.5"
                  >
                    ×
                  </button>
                </div>
                {v.alsoIn.length > 0 && (
                  <div className="text-[9px] font-mono text-tx3 mt-0.5">
                    {t('projects.review.alsoIn', { names: v.alsoIn.map((id) => getTemplate(id)?.label ?? id).join(', ') })}
                  </div>
                )}
                {bad && (
                  <div className="text-[9px] font-mono text-danger mt-0.5">
                    {v.key === '' || !isValidEnvKey(v.key) ? t('projects.review.invalidKey') : t('projects.review.duplicateKey')}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      ))}
      <div className="text-[10px] text-tx3 font-mono mt-1">{t('projects.review.placeholderNote', { placeholder: TEMPLATE_PLACEHOLDER })}</div>
    </div>
  );
}

// ─── DeleteProjectModal ─────────────────────────────────────────────────────────

function DeleteProjectModal({
  project,
  onCancel,
  onConfirm,
}: {
  project:   Project;
  onCancel:  () => void;
  onConfirm: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const previewDelete = useProjectStore((s) => s.previewDelete);
  const [impact, setImpact]   = useState<ProjectDeleteImpact | null>(null);
  const [typed, setTyped]     = useState('');
  const [deleting, setDeleting] = useState(false);

  const required = t('projects.deleteModal.confirmPhrase', { name: project.name });
  const matches = typed.trim().toLowerCase() === required.toLowerCase();

  useEffect(() => {
    previewDelete(project.id).then(setImpact).catch(() => setImpact(null));
  }, [project.id]);

  return (
    <div className="absolute inset-0 bg-black/70 flex items-center justify-center z-30 p-4">
      <div className="w-full max-w-sm bg-surface border border-danger rounded-[4px] p-4">
        <div className="text-[14px] font-bold text-tx mb-2">{t('projects.deleteModal.title', { name: project.name })}</div>

        {impact ? (
          <div className="text-[12px] text-tx3 mb-3 leading-[1.6]">
            {t('projects.deleteModal.removesPrefix')}<strong className="text-tx">{impact.environments}</strong>{t(impact.environments === 1 ? 'projects.deleteModal.envWord_one' : 'projects.deleteModal.envWord_other')}
            {impact.itemsDeleted > 0 && (
              <>{t('projects.deleteModal.deletesPrefix')}<strong className="text-danger">{impact.itemsDeleted}</strong>{t(impact.itemsDeleted === 1 ? 'projects.deleteModal.localWord_one' : 'projects.deleteModal.localWord_other')}</>
            )}{t('projects.deleteModal.period')}
            {impact.itemsOrphaned > 0 && (
              <>{' '}<strong className="text-accent">{impact.itemsOrphaned}</strong>{t(impact.itemsOrphaned === 1 ? 'projects.deleteModal.preserved_one' : 'projects.deleteModal.preserved_other')}</>
            )}
          </div>
        ) : (
          <div className="text-[11px] text-tx3 mb-3 font-mono">{t('projects.deleteModal.calculating')}</div>
        )}

        <div className="text-[11px] text-tx3 mb-1">
          {t('projects.deleteModal.typePrefix')}<span className="font-mono text-tx">{required}</span>{t('projects.deleteModal.typeSuffix')}
        </div>
        <input
          value={typed}
          onChange={(e) => setTyped(e.target.value)}
          placeholder={required}
          className="w-full bg-bg border border-bd2 text-tx font-mono text-[12px] rounded-[3px] px-3 py-[7px] outline-none focus:border-danger mb-3"
        />

        <div className="flex gap-2">
          <button onClick={onCancel}
            className="flex-1 py-2 bg-transparent border border-bd2 rounded-[3px] text-tx2 text-[12px] cursor-pointer font-ui">
            {t('common.cancel')}
          </button>
          <button
            onClick={async () => { setDeleting(true); try { await onConfirm(); } finally { setDeleting(false); } }}
            disabled={!matches || deleting}
            className="flex-1 py-2 bg-danger border-none rounded-[3px] text-white text-[12px] font-bold cursor-pointer font-ui disabled:opacity-40"
          >
            {deleting ? t('projects.deleteModal.deleting') : t('common.delete')}
          </button>
        </div>
      </div>
    </div>
  );
}

// ─── InjectConfirmModal ─────────────────────────────────────────────────────────
// Shown before an inject that would overwrite one or more paths not created
// by crypt-env (as reported by `environment_inject_preview`). Naming the
// exact files is the point — the human in front of the GUI is the only
// surface where that's possible (API/MCP callers get a hard 409 instead).

function InjectConfirmModal({
  foreign,
  onCancel,
  onConfirm,
}: {
  foreign:   string[];
  onCancel:  () => void;
  onConfirm: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const [writing, setWriting] = useState(false);
  const one = foreign.length === 1;

  return (
    <div className="absolute inset-0 bg-black/70 flex items-center justify-center z-30 p-4">
      <div className="w-full max-w-sm bg-surface border border-accent-d rounded-[4px] p-4">
        <div className="text-[14px] font-bold text-tx mb-2">
          {t(one ? 'projects.injectModal.title_one' : 'projects.injectModal.title_other')}
        </div>
        <div className="text-[12px] text-tx3 mb-3 leading-[1.6]">
          {t(one ? 'projects.injectModal.notManaged_one' : 'projects.injectModal.notManaged_other')}
          {' '}{t('projects.injectModal.bakPrefix')}<span className="font-mono text-tx">.bak</span>{t('projects.injectModal.bakSuffix')}
        </div>
        <ul className="mb-3 max-h-32 overflow-y-auto space-y-1">
          {foreign.map((p) => (
            <li key={p} className="text-[11px] font-mono text-tx bg-bg border border-bd rounded-[2px] px-2 py-1 truncate">
              {p}
            </li>
          ))}
        </ul>
        <div className="flex gap-2">
          <button onClick={onCancel}
            className="flex-1 py-2 bg-transparent border border-bd2 rounded-[3px] text-tx2 text-[12px] cursor-pointer font-ui">
            {t('common.cancel')}
          </button>
          <button
            onClick={async () => { setWriting(true); try { await onConfirm(); } finally { setWriting(false); } }}
            disabled={writing}
            className="flex-1 py-2 bg-accent border-none rounded-[3px] text-[#020504] text-[12px] font-bold cursor-pointer font-ui disabled:opacity-40"
          >
            {writing ? t('projects.injectModal.writing') : t('projects.injectModal.overwrite')}
          </button>
        </div>
      </div>
    </div>
  );
}

// ─── VarRow (project detail: real vault item, type-aware) ─────────────────────

function varSummary(item: VaultItem, reveal: boolean): string {
  if (item.type === 'secret')     return reveal ? item.value : '••••••••';
  if (item.type === 'credential') return `${item.username} / ${reveal ? item.password : '••••••••'}`;
  if (item.type === 'link')       return item.url;
  if (item.type === 'command')    return item.command;
  if (item.type === 'note')       return item.content.length > 48 ? item.content.slice(0, 48) + '…' : item.content;
  return '';
}

function VarRow({
  v,
  item,
  onUnlink,
  onDeleteEverywhere,
}: {
  v:                  EnvironmentVar;
  item:                VaultItem | undefined;
  onUnlink:            () => void;
  onDeleteEverywhere:  () => void;
}) {
  const { t } = useTranslation();
  const [reveal, setReveal] = useState(false);

  if (!item) {
    return (
      <div className="py-2 border-b border-bd flex items-center gap-2">
        <span className="flex-1 text-[11px] font-mono text-danger truncate">{v.key} — {t('projects.varRow.itemMissing')}</span>
        <button onClick={onUnlink} className="text-tx3 hover:text-danger transition-colors shrink-0">
          <Icon name="trash" size={13} />
        </button>
      </div>
    );
  }

  return (
    <div className="py-2 border-b border-bd group">
      <div className="flex items-center gap-2 mb-1">
        <span className="flex-1 font-mono text-[12px] text-tx truncate">{v.key}</span>
        <span className="text-[9px] font-mono text-tx3 bg-bg border border-bd px-1.5 py-[1px] rounded-[2px] uppercase shrink-0">
          {item.type}
        </span>
        {item.isGlobal && (
          <span className="text-[9px] font-mono text-accent bg-accent-b border border-accent-d px-1.5 py-[1px] rounded-[2px] shrink-0">
            {t('projects.varRow.global')}
          </span>
        )}
      </div>
      <div className="flex items-center gap-2 pl-0">
        <span className="flex-1 font-mono text-[11px] text-tx3 truncate">{varSummary(item, reveal)}</span>
        {(item.type === 'secret' || item.type === 'credential') && (
          <button onClick={() => setReveal((r) => !r)} className="text-tx3 hover:text-tx transition-colors shrink-0">
            <Icon name={reveal ? 'eyeOff' : 'eye'} size={12} />
          </button>
        )}
        <button onClick={onUnlink} title={t('projects.varRow.unlink')}
          className="text-tx3 hover:text-tx transition-colors shrink-0 opacity-0 group-hover:opacity-100">
          <Icon name="close" size={12} />
        </button>
        <button onClick={onDeleteEverywhere} title={t('projects.varRow.deleteEverywhere')}
          className="text-tx3 hover:text-danger transition-colors shrink-0 opacity-0 group-hover:opacity-100">
          <Icon name="trash" size={12} />
        </button>
      </div>
    </div>
  );
}

// ─── AddVarPanel (create a project-scoped typed item, or import a global one) ──

export function AddVarPanel({
  project,
  globalItems,
  onAdded,
  onCancel,
}: {
  project:     Project;
  globalItems: { id: number; name: string }[];
  onAdded:     (v: EnvironmentVar) => void;
  onCancel:    () => void;
}) {
  const { t } = useTranslation();
  const showToast        = useVaultStore((s) => s.showToast);
  const createProjectItem = useProjectStore((s) => s.createProjectItem);

  const [mode, setMode]   = useState<'choose' | 'new' | 'import'>('choose');
  const [type, setType]   = useState<ItemType>('secret');
  const [key, setKey]     = useState('');
  const [fields, setFields] = useState<ItemFieldsState>(emptyItemFields());
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [showVal, setShowVal] = useState(false);
  const [saving, setSaving] = useState(false);
  const [importId, setImportId] = useState<number | null>(globalItems[0]?.id ?? null);
  // 'new' mode: the vault name follows the KEY until edited by hand; picking a
  // matching global secret links it instead of creating a new item.
  const [nameTouched, setNameTouched] = useState(false);
  const [linkedId, setLinkedId] = useState<number | null>(null);

  const nameKey   = itemNameKey(type);
  const vaultName = fields[nameKey];
  const linked    = linkedId == null ? undefined : globalItems.find((i) => i.id === linkedId);
  const matches   = useMemo(() => {
    const q = vaultName.trim().toLowerCase();
    if (!q || linked) return [];
    return globalItems.filter((i) => i.name.toLowerCase().includes(q)).slice(0, 5);
  }, [globalItems, vaultName, linked]);

  const handleKeyChange = (v: string) => {
    setKey(v);
    if (!nameTouched && !linked) { set(nameKey, v); clearError(nameKey); }
  };

  const handleTypeChange = (ty: ItemType) => {
    const carried = vaultName;
    setType(ty);
    setErrors({});
    setFields((f) => ({ ...f, [itemNameKey(ty)]: carried }));
  };

  const pickExisting = (item: { id: number; name: string }) => {
    setLinkedId(item.id);
    if (!key.trim()) setKey(item.name);
    setErrors({});
  };

  const set = (k: keyof ItemFieldsState, v: string) => setFields((f) => ({ ...f, [k]: v }));
  const clearError = (k: string) => setErrors((r) => ({ ...r, [k]: '' }));

  const handleCreate = async () => {
    if (!key.trim()) { showToast(t('projects.toast.keyRequired'), 'error'); return; }
    if (linked) {
      onAdded({ id: -Date.now(), key: key.trim().toUpperCase().replace(/[^A-Z0-9_]/g, ''), itemId: linked.id });
      return;
    }
    const e = validateItemFields(type, fields);
    if (Object.keys(e).length) { setErrors(e); return; }
    setSaving(true);
    try {
      const item = await createProjectItem(project.id, { ...fields, type, categories: [], isGlobal: false } as Omit<VaultItem, 'id' | 'created'>);
      // The var row resolves its item from the vault store — refresh before
      // linking, or the new row renders as "item missing" until a reload.
      await refreshVaultItems();
      onAdded({ id: -Date.now(), key: key.trim().toUpperCase().replace(/[^A-Z0-9_]/g, ''), itemId: item.id });
    } catch (err) {
      reportError(err);
    } finally {
      setSaving(false);
    }
  };

  const handleImport = () => {
    if (importId == null) { showToast(t('projects.toast.pickGlobalFirst'), 'error'); return; }
    const found = globalItems.find((i) => i.id === importId);
    const derivedKey = key.trim() || found?.name || '';
    onAdded({ id: -Date.now(), key: derivedKey.toUpperCase().replace(/[^A-Z0-9_]/g, ''), itemId: importId });
  };

  if (mode === 'choose') {
    return (
      <div className="border border-bd2 rounded-[3px] p-3 mb-2 bg-raised">
        <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.06em] mb-2">{t('projects.addVar.header')}</div>
        <div className="flex gap-2">
          <button onClick={() => setMode('new')}
            className="flex-1 flex items-center justify-center gap-1 py-2 rounded-[3px] text-[11px] font-bold font-ui text-accent border border-accent-d hover:bg-accent-b transition-colors">
            <Icon name="plus" size={11} />{t('projects.addVar.newItem')}
          </button>
          <button onClick={() => setMode('import')} disabled={globalItems.length === 0}
            title={globalItems.length === 0 ? t('projects.addVar.noGlobals') : undefined}
            className="flex-1 flex items-center justify-center gap-1 py-2 rounded-[3px] text-[11px] font-bold font-ui text-tx2 border border-bd2 hover:text-tx transition-colors disabled:opacity-40">
            <Icon name="shield" size={11} />{t('projects.addVar.importGlobal')}
          </button>
        </div>
        <button onClick={onCancel} className="w-full mt-2 text-[10px] font-ui text-tx3 hover:text-tx transition-colors">
          {t('projects.addVar.cancel')}
        </button>
      </div>
    );
  }

  if (mode === 'import') {
    return (
      <div className="border border-bd2 rounded-[3px] p-3 mb-2 bg-raised">
        <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.06em] mb-2">{t('projects.addVar.importHeader')}</div>
        <div className="mb-2">
          <select
            value={importId ?? ''}
            onChange={(e) => setImportId(parseInt(e.target.value, 10))}
            className="w-full bg-bg border border-bd2 text-tx rounded-[3px] px-2 py-[6px] text-[12px] font-ui cursor-pointer outline-none focus:border-accent-d"
          >
            {globalItems.map((i) => <option key={i.id} value={i.id}>{i.name}</option>)}
          </select>
        </div>
        <input
          value={key}
          onChange={(e) => setKey(e.target.value)}
          placeholder={t('projects.addVar.keyPlaceholderImport')}
          className="w-full bg-bg border border-bd2 text-tx font-mono text-[12px] rounded-[3px] px-2 py-[6px] outline-none focus:border-accent-d mb-2"
        />
        <div className="flex gap-2">
          <button onClick={() => setMode('choose')} className="flex-1 py-2 rounded-[3px] text-[11px] font-ui text-tx2 border border-bd2 hover:text-tx transition-colors">
            {t('projects.addVar.back')}
          </button>
          <button onClick={handleImport} className="flex-1 py-2 rounded-[3px] text-[11px] font-bold font-ui text-accent border border-accent-d hover:bg-accent-b transition-colors">
            {t('projects.addVar.add')}
          </button>
        </div>
      </div>
    );
  }

  // mode === 'new'
  return (
    <div className="border border-bd2 rounded-[3px] p-3 mb-2 bg-raised">
      <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.06em] mb-2">{t('projects.addVar.newHeader')}</div>
      <div className="border border-accent/60 focus-within:border-accent ring-1 ring-accent/20 rounded-[3px] bg-accent-b p-2 mb-3">
        <label htmlFor="add-var-key" className="flex items-center gap-1.5 mb-1">
          <span className="text-[9px] font-mono font-bold tracking-[0.1em] text-[#020504] bg-accent rounded-[2px] px-1.5 py-[1px]">
            {t('projects.addVar.keyBadge')}
          </span>
          <span className="text-[10px] font-mono text-accent">{t('projects.addVar.keyRequired')}</span>
        </label>
        <input
          id="add-var-key"
          value={key}
          onChange={(e) => handleKeyChange(e.target.value)}
          placeholder={t('projects.addVar.keyPlaceholder')}
          className="w-full bg-bg border border-accent-d text-tx font-mono text-[13px] font-semibold rounded-[3px] px-2 py-[6px] outline-none focus:border-accent"
        />
        <div className="text-[10px] text-tx3 font-mono mt-1">{t('projects.addVar.keyHelp')}</div>
      </div>
      <F>
        <Label label={t('projects.addVar.vaultNameLabel')} err={errors[nameKey]} />
        {linked ? (
          <div className="flex items-center gap-2 border border-accent-d bg-accent-b rounded-[3px] px-[10px] py-2">
            <Icon name="shield" size={11} />
            <span className="flex-1 min-w-0 text-[12px] font-mono text-accent truncate">{linked.name}</span>
            <button onClick={() => setLinkedId(null)} className="text-[10px] font-ui text-tx3 hover:text-tx transition-colors shrink-0">
              {t('projects.addVar.unlinkExisting')}
            </button>
          </div>
        ) : (
          <input
            value={vaultName}
            onChange={(e) => { set(nameKey, e.target.value); setNameTouched(true); clearError(nameKey); }}
            placeholder={t('projects.addVar.vaultNamePlaceholder')}
            className={`w-full px-[10px] py-2 text-[12px] font-mono bg-raised border rounded-[3px] text-tx placeholder:text-tx3 outline-none focus:border-accent-d ${errors[nameKey] ? 'border-danger' : 'border-bd2'}`}
          />
        )}
        <div className="text-[10px] text-tx3 font-mono mt-1">
          {linked ? t('projects.addVar.linkedExisting') : t('projects.addVar.vaultNameHelp')}
        </div>
        {matches.length > 0 && (
          <div className="mt-1.5 border border-bd2 rounded-[3px] bg-bg">
            <div className="text-[9px] font-mono tracking-[0.1em] text-tx3 px-2 pt-1.5 pb-1">{t('projects.addVar.matchesHeader')}</div>
            {matches.map((m) => (
              <button key={m.id} onClick={() => pickExisting(m)}
                className="w-full flex items-center gap-2 px-2 py-1.5 text-left text-[11px] font-mono text-tx2 hover:bg-raised hover:text-accent transition-colors">
                <Icon name="shield" size={10} />
                <span className="truncate">{m.name}</span>
              </button>
            ))}
          </div>
        )}
      </F>
      {!linked && (
        <>
          <ItemTypePicker type={type} onSelect={handleTypeChange} />
          <ItemTypeFields
            type={type}
            form={fields}
            errors={errors}
            showVal={showVal}
            setShowVal={setShowVal}
            set={set}
            clearError={clearError}
            hideName
          />
        </>
      )}
      <div className="flex gap-2 mt-1">
        <button onClick={() => setMode('choose')} className="flex-1 py-2 rounded-[3px] text-[11px] font-ui text-tx2 border border-bd2 hover:text-tx transition-colors">
          {t('projects.addVar.back')}
        </button>
        <button onClick={handleCreate} disabled={saving} className="flex-1 py-2 rounded-[3px] text-[11px] font-bold font-ui text-accent border border-accent-d hover:bg-accent-b transition-colors disabled:opacity-40">
          {saving ? t('common.saving') : t('projects.addVar.add')}
        </button>
      </div>
    </div>
  );
}

// ─── Small helpers ────────────────────────────────────────────────────────────

function truncatePath(p: string): string {
  const norm = p.replace(/\\/g, '/');
  return norm.length > 38 ? '…' + norm.slice(-37) : norm;
}

function folderNameFromPath(p: string): string {
  const normalized = p.replace(/\\/g, '/').replace(/\/[^/]+$/, '');
  const parts = normalized.split('/').filter(Boolean);
  return parts[parts.length - 1] ?? '';
}

async function refreshVaultItems() {
  try {
    const result = await invoke<{ items: VaultItem[]; categories: Category[] }>('vault_list');
    useVaultStore.setState({ items: result.items, cats: result.categories });
  } catch {
    // best-effort refresh; the next natural reload will pick up any drift
  }
}

// ─── ProjectCard (list view) ───────────────────────────────────────────────────

function ProjectCard({ project, cats, onOpen }: { project: Project; cats: Category[]; onOpen: () => void }) {
  const { t } = useTranslation();
  const envNames = project.environments.map((e) => e.name);
  const tagColor = (name: string) => cats.find((c) => c.name === name)?.color;
  return (
    <button
      onClick={onOpen}
      className="w-full text-left px-3.5 py-3 border-b border-bd hover:bg-raised transition-colors"
    >
      <div className="flex items-center gap-2 mb-1.5">
        <span className="flex-1 text-[13px] font-semibold text-tx font-ui truncate">{project.name}</span>
        <span className="text-[9px] font-mono text-tx3 bg-bg border border-bd px-1.5 py-[1px] rounded-[2px] uppercase shrink-0">
          {templateLabels(project.template).join(' · ')}
        </span>
      </div>
      <div className="flex items-center gap-1 mb-1.5 flex-wrap min-h-[18px]">
        {envNames.length === 0 ? (
          <span className="text-[10px] font-mono text-tx3 italic">{t('projects.card.noEnvironments')}</span>
        ) : (
          envNames.map((n) => (
            <span key={n} className="text-[10px] font-mono px-1.5 py-[1px] bg-bg border border-bd2 text-tx3 rounded-[2px] truncate">
              {n}
            </span>
          ))
        )}
      </div>
      {project.categories.length > 0 && (
        <div className="flex items-center gap-1 flex-wrap">
          {project.categories.map((c) => (
            <span key={c} className="flex items-center gap-1 text-[10px] font-ui px-1.5 py-[1px] bg-accent-b border border-accent-d text-accent rounded-[2px] truncate">
              <span className="w-[5px] h-[5px] rounded-full shrink-0" style={{ background: tagColor(c) ?? 'currentColor' }} />
              {c}
            </span>
          ))}
        </div>
      )}
    </button>
  );
}

// ─── EnvironmentCard (project detail view) ─────────────────────────────────────

function EnvironmentCard({
  env,
  onOpen,
  onInject,
}: {
  env:      Environment;
  onOpen:   () => void;
  onInject: (id: number) => Promise<InjectResult>;
}) {
  const { t } = useTranslation();
  const [injectState, setInjectState] = useState<'idle' | 'ok' | 'err'>('idle');

  const chips     = env.vars.slice(0, 4);
  const overflow  = env.vars.length - 4;
  const firstPath = env.paths[0];
  const canInject = env.paths.length > 0 && env.vars.length > 0;

  const handleInjectClick = async (e: React.MouseEvent) => {
    e.stopPropagation();
    if (!canInject || injectState !== 'idle') return;
    try {
      await onInject(env.id);
      setInjectState('ok');
      setTimeout(() => setInjectState('idle'), 2000);
    } catch (err) {
      // A cancelled confirm-overwrite dialog isn't a failure — just reset.
      if (String(err) === 'Error: cancelled') {
        setInjectState('idle');
        return;
      }
      setInjectState('err');
      setTimeout(() => setInjectState('idle'), 2000);
    }
  };

  return (
    <button
      onClick={onOpen}
      className="w-full text-left px-3.5 py-3 border-b border-bd hover:bg-raised transition-colors"
    >
      <div className="flex items-center gap-2 mb-1.5">
        <span className="flex-1 text-[13px] font-semibold text-tx font-ui truncate">{env.name}</span>
        {env.isDefault && (
          <span className="text-[9px] font-mono text-accent bg-accent-b border border-accent-d px-1.5 py-[1px] rounded-[2px] uppercase shrink-0">
            {t('projects.envCard.default')}
          </span>
        )}
      </div>

      <div className="flex items-center gap-1 mb-2 flex-wrap min-h-[18px]">
        {env.vars.length === 0 ? (
          <span className="text-[10px] font-mono text-tx3 italic">{t('projects.envCard.noVariables')}</span>
        ) : (
          <>
            {chips.map((v) => (
              <span key={v.id} className="text-[10px] font-mono px-1.5 py-[1px] bg-bg border border-bd2 text-tx3 rounded-[2px] max-w-[9rem] truncate">
                {v.key || '…'}
              </span>
            ))}
            {overflow > 0 && <span className="text-[10px] font-mono text-tx3">+{overflow}</span>}
          </>
        )}
      </div>

      <div className="flex items-center gap-2">
        <span className="flex-1 text-[10px] font-mono text-tx3 truncate">
          {firstPath ? truncatePath(firstPath) : <span className="opacity-50">{t('projects.envCard.noPath')}</span>}
        </span>
        <button
          onClick={handleInjectClick}
          disabled={!canInject || injectState !== 'idle'}
          title={!canInject ? t('projects.envCard.injectDisabled') : undefined}
          className={[
            'shrink-0 text-[9px] font-mono font-bold px-2 py-[3px] rounded-[2px] border transition-colors',
            injectState === 'ok'
              ? 'border-transparent text-tx3 cursor-default'
              : injectState === 'err'
              ? 'border-transparent text-danger cursor-default'
              : canInject
              ? 'border-accent-d text-accent bg-accent-b hover:opacity-80 cursor-pointer'
              : 'border-bd text-tx3 opacity-40 cursor-not-allowed',
          ].join(' ')}
        >
          {injectState === 'ok' ? t('projects.envCard.injected') : injectState === 'err' ? t('projects.envCard.failed') : t('projects.envCard.inject')}
        </button>
      </div>
    </button>
  );
}

// ─── ProjectManager ─────────────────────────────────────────────────────────────

type Mode = 'projects' | 'project' | 'environment';

export function ProjectManager() {
  const { t } = useTranslation();
  const go             = useVaultStore((s) => s.go);
  const showToast      = useVaultStore((s) => s.showToast);
  const items          = useVaultStore((s) => s.items);
  const cats           = useVaultStore((s) => s.cats);
  const getItemOwners  = useVaultStore((s) => s.getItemOwners);
  const saveCats       = useVaultStore((s) => s.saveCats);

  const { projects, loading, load, saveProject, createFromTemplates, removeProject, saveEnvironment, removeEnvironment, inject, previewInject } =
    useProjectStore();

  // Pending confirm-then-inject flow (see `runInject` below): `injectConfirm`
  // holds the modal's display data, while the resolve/reject pair for the
  // promise `runInject` returned to its caller lives in a ref so it survives
  // re-renders without becoming React state itself.
  const [injectConfirm, setInjectConfirm] = useState<{ id: number; foreign: string[] } | null>(null);
  const pendingInjectRef = useRef<{ resolve: (r: InjectResult) => void; reject: (e: unknown) => void } | null>(null);

  // Single entry point for both the project-list quick-inject button and the
  // environment editor's INJECT button: previews first, and only prompts for
  // confirmation when the preview reports a path crypt-env doesn't manage.
  const runInject = async (id: number): Promise<InjectResult> => {
    const preview = await previewInject(id);
    if (preview.foreign.length === 0) return inject(id, false);
    return new Promise<InjectResult>((resolve, reject) => {
      pendingInjectRef.current = { resolve, reject };
      setInjectConfirm({ id, foreign: preview.foreign });
    });
  };

  const confirmPendingInject = async () => {
    if (!injectConfirm) return;
    const { id } = injectConfirm;
    const pending = pendingInjectRef.current;
    pendingInjectRef.current = null;
    setInjectConfirm(null);
    try {
      const result = await inject(id, true);
      pending?.resolve(result);
    } catch (e) {
      pending?.reject(e);
    }
  };

  const cancelPendingInject = () => {
    const pending = pendingInjectRef.current;
    pendingInjectRef.current = null;
    setInjectConfirm(null);
    pending?.reject(new Error('cancelled'));
  };

  const [mode, setMode] = useState<Mode>('projects');
  const [selectedProject, setSelectedProject] = useState<Project | null>(null);
  const [selectedEnv, setSelectedEnv] = useState<Environment | null>(null);

  // Landing page search/filter
  const [query, setQuery] = useState('');
  const [tagFilter, setTagFilter] = useState<Set<string>>(new Set());
  const [tagOpen, setTagOpen] = useState(false);

  // Project form state
  const [projName,        setProjName]        = useState('');
  const [projDescription, setProjDescription] = useState('');
  const [projTemplate,    setProjTemplate]    = useState<ProjectTemplate>('generic');
  const [projTemplateIds, setProjTemplateIds] = useState<string[]>([]);
  const [projInitialEnv,  setProjInitialEnv]  = useState('');
  const [templateVars,    setTemplateVars]    = useState<ReviewVar[]>([]);
  const [projCategories,  setProjCategories]  = useState<string[]>([]);
  const [isCreatingProj,  setIsCreatingProj]  = useState(false);
  const [templateModal,   setTemplateModal]   = useState(false);
  const [confirmDelProj,  setConfirmDelProj]  = useState(false);
  const [shareModalMode,  setShareModalMode]  = useState<ProjectShareMode | null>(null);

  // Environment form state
  const [envName,       setEnvName]       = useState('');
  const [envIsDefault,  setEnvIsDefault]  = useState(false);
  const [envPaths,      setEnvPaths]      = useState<string[]>([]);
  const [newPath,       setNewPath]       = useState('');
  const [envVars,       setEnvVars]       = useState<EnvironmentVar[]>([]);
  const [isCreatingEnv, setIsCreatingEnv] = useState(false);
  const [confirmDelEnv, setConfirmDelEnv] = useState(false);
  const [addingVar,     setAddingVar]     = useState(false);

  const [saving,    setSaving]    = useState(false);
  const [injecting, setInjecting] = useState(false);

  // WSL bridge (issue #3) — ephemeral machine state, not vault state, so a
  // plain local state is enough (no Zustand store, no TanStack Query).
  const [isWindows,      setIsWindows]      = useState(false);
  const [wslDistros,     setWslDistros]     = useState<string[]>([]);
  const [wslBusy,        setWslBusy]        = useState(false);
  const [wslPickerOpen,  setWslPickerOpen]  = useState(false);

  useEffect(() => {
    setIsWindows(platform() === 'windows');
  }, []);

  useEffect(() => {
    if (!isWindows) return;
    invoke<string[]>('wsl_list_distros')
      .then(setWslDistros)
      .catch(() => setWslDistros([]));
  }, [isWindows]);

  useEffect(() => { load(); }, []);

  // Keep the selected project/environment in sync once the store reloads
  useEffect(() => {
    if (!selectedProject) return;
    const fresh = projects.find((p) => p.id === selectedProject.id);
    if (fresh) setSelectedProject(fresh);
  }, [projects]);

  useEffect(() => {
    if (!selectedProject || !selectedEnv) return;
    const fresh = selectedProject.environments.find((e) => e.id === selectedEnv.id);
    if (fresh) setSelectedEnv(fresh);
  }, [selectedProject]);

  // Only items explicitly marked "global" are offered for import — that's
  // what makes a secret importable across projects/environments.
  const globalItems = useMemo(() => items
    .filter((i) => i.isGlobal)
    .map((i) => ({
      id:   i.id,
      name: ('name' in i ? i.name : 'title' in i ? (i as any).title : '') || `#${i.id}`,
    })), [items]);

  const itemsById = useMemo(() => new Map(items.map((i) => [i.id, i])), [items]);

  const filteredProjects = useMemo(() => projects.filter((p) => {
    if (tagFilter.size > 0 && !p.categories.some((c) => tagFilter.has(c))) return false;
    const q = query.trim().toLowerCase();
    if (!q) return true;
    return p.name.toLowerCase().includes(q) || (p.description ?? '').toLowerCase().includes(q);
  }), [projects, query, tagFilter]);

  // ── Navigation ──

  const goToProjects = () => {
    setMode('projects');
    setSelectedProject(null);
    setSelectedEnv(null);
    setIsCreatingProj(false);
    setConfirmDelProj(false);
  };

  const openProject = (p: Project) => {
    setSelectedProject(p);
    setProjName(p.name);
    setProjDescription(p.description ?? '');
    setProjTemplate(p.template as ProjectTemplate);
    setProjCategories(p.categories);
    setIsCreatingProj(false);
    setConfirmDelProj(false);
    setMode('project');
  };

  const openEnvironment = (env: Environment) => {
    setSelectedEnv(env);
    setEnvName(env.name);
    setEnvIsDefault(env.isDefault);
    setEnvPaths(env.paths);
    setNewPath('');
    setEnvVars(env.vars);
    setIsCreatingEnv(false);
    setConfirmDelEnv(false);
    setAddingVar(false);
    setMode('environment');
  };

  const backToProject = () => {
    setMode('project');
    setSelectedEnv(null);
    setIsCreatingEnv(false);
    setConfirmDelEnv(false);
  };

  // ── Project actions ──

  const handleNewProject = () => {
    setSelectedProject(null);
    setProjName('');
    setProjDescription('');
    setProjTemplate('generic');
    setProjTemplateIds([]);
    setTemplateVars([]);
    setProjInitialEnv('');
    setProjCategories([]);
    setIsCreatingProj(true);
    setConfirmDelProj(false);
    setTemplateModal(true);
  };

  // Re-picking templates rebuilds the variable list from scratch (edits are
  // discarded) but only ever adds categories, never drops user-picked ones.
  const handleTemplateSelect = (ids: string[]) => {
    setProjTemplateIds(ids);
    setProjTemplate(templateString(ids));
    setTemplateVars(toReviewVars(mergeTemplateVars(ids)));
    setProjCategories((cur) => [...cur, ...templateCategories(ids).filter((c) => !cur.includes(c))]);
    setTemplateModal(false);
    setMode('project');
  };

  // Blank is fine (backend defaults to "default"); otherwise mirror the env-name rule.
  const initialEnvValid = projInitialEnv.trim() === '' || isValidEnvironmentName(projInitialEnv.trim());

  const templateVarsInvalid = useMemo(() => invalidReviewKeys(templateVars).size > 0, [templateVars]);

  // Categories referenced by name must exist or the backend silently drops them.
  const ensureCategories = async (names: string[]) => {
    const missing = names.filter((n) => !cats.some((c) => c.name === n));
    if (missing.length === 0) return;
    const now = Date.now();
    const added = missing.map((name, i) => ({
      id:    `c${now}${i}`,
      name,
      color: CAT_COLORS_PRESET[(cats.length + i) % CAT_COLORS_PRESET.length],
    }));
    await saveCats([...cats, ...added]);
  };

  // Inline tag creation (TagInput): persist, then select. Rethrows so the
  // input keeps the draft on failure.
  const handleCreateCategory = async (name: string) => {
    try {
      await ensureCategories([name]);
      setProjCategories((cur) => (cur.includes(name) ? cur : [...cur, name]));
    } catch (e) {
      reportError(e);
      throw e;
    }
  };

  const handleCreateProject = async () => {
    if (!isValidProjectName(projName.trim())) { showToast(t('projects.invalidName', { rule: t('projects.projectNameRule') }), 'error'); return; }
    if (templateVarsInvalid || !initialEnvValid) return;
    setSaving(true);
    try {
      await ensureCategories(projCategories);
      const id = await createFromTemplates({
        name:        projName.trim(),
        description: projDescription || undefined,
        template:    templateString(projTemplateIds),
        categories:  projCategories,
        initialEnvironment: projInitialEnv.trim() || undefined,
        vars:        templateVars.map((v) => ({ key: v.key, value: v.value })),
      });
      await refreshVaultItems();
      const fresh = useProjectStore.getState().projects.find((p) => p.id === id);
      if (fresh) {
        setIsCreatingProj(false);
        setTemplateVars([]);
        openProject(fresh);
      }
      showToast(t('projects.toast.projectSaved'));
    } catch (e) {
      reportError(e);
    } finally {
      setSaving(false);
    }
  };

  const handleSaveProject = async () => {
    if (!isValidProjectName(projName.trim())) { showToast(t('projects.invalidName', { rule: t('projects.projectNameRule') }), 'error'); return; }
    setSaving(true);
    try {
      const id = await saveProject({
        id:          selectedProject?.id,
        name:        projName.trim(),
        description: projDescription || undefined,
        template:    projTemplate,
        categories:  projCategories,
      });
      const fresh = useProjectStore.getState().projects.find((p) => p.id === id);
      if (fresh) {
        setIsCreatingProj(false);
        openProject(fresh);
      }
      showToast(t('projects.toast.projectSaved'));
    } catch (e) {
      reportError(e);
    } finally {
      setSaving(false);
    }
  };

  const handleDeleteProject = async () => {
    if (!selectedProject) return;
    await removeProject(selectedProject.id);
    await refreshVaultItems();
    setConfirmDelProj(false);
    goToProjects();
    showToast(t('projects.toast.projectDeleted'));
  };

  const handleExportProject = async () => {
    if (!selectedProject) return;
    try {
      await invoke('project_export', { projectId: selectedProject.id });
      showToast(t('projects.toast.templateSaved'));
    } catch (e) {
      if (String(e) !== 'cancelled') reportError(e);
    }
  };

  const handleImportProject = async () => {
    try {
      const imported = await invoke<ExportedProject>('project_import');
      setSaving(true);
      const projectId = await saveProject({
        name:        imported.name,
        description: imported.description,
        template:    imported.template,
        categories:  [],
      });
      showToast(t('projects.toast.imported', { name: imported.name }));
      const fresh = useProjectStore.getState().projects.find((p) => p.id === projectId);
      if (fresh) openProject(fresh);
    } catch (e) {
      if (String(e) !== 'cancelled') reportError(e);
    } finally {
      setSaving(false);
    }
  };

  // ── Environment actions ──

  const handleNewEnvironment = () => {
    if (!selectedProject) return;
    setSelectedEnv(null);
    setEnvName('');
    setEnvIsDefault(selectedProject.environments.length === 0);
    setEnvPaths([]);
    setNewPath('');
    setEnvVars([]);
    setIsCreatingEnv(true);
    setConfirmDelEnv(false);
    setAddingVar(false);
    setMode('environment');
  };

  const addPath = () => {
    const trimmed = newPath.trim();
    if (!trimmed || envPaths.includes(trimmed)) return;
    setEnvPaths((prev) => [...prev, trimmed]);
    setNewPath('');
  };

  const removePath = (idx: number) => {
    setEnvPaths((prev) => prev.filter((_, i) => i !== idx));
  };

  // Shared tail of "a path was picked" handling, reused by both the plain
  // browse button and the WSL-seeded one (plan §3.4).
  const applyPickedEnvPath = (picked: string | null) => {
    if (!picked) return;
    const trimmed = picked.trim();
    if (!envPaths.includes(trimmed)) setEnvPaths((prev) => [...prev, trimmed]);
    if (!projName && isCreatingProj) {
      const folder = folderNameFromPath(trimmed);
      if (folder) setProjName(folder);
    }
  };

  const handlePickEnvPath = async () => {
    try {
      const picked = await invoke<string | null>('project_pick_env_path');
      applyPickedEnvPath(picked);
    } catch (e) {
      reportError(e);
    }
  };

  const handleBrowseWsl = async (distro: string) => {
    setWslPickerOpen(false);
    setWslBusy(true);
    try {
      const home = await invoke<string>('wsl_distro_home', { distro });
      const picked = await invoke<string | null>('project_pick_env_path', { startDir: home });
      applyPickedEnvPath(picked);
    } catch (e) {
      reportError(e);
    } finally {
      setWslBusy(false);
    }
  };

  const handleUnlinkVar = (idx: number) => {
    setEnvVars((prev) => prev.filter((_, i) => i !== idx));
  };

  const handleDeleteVarEverywhere = async (v: EnvironmentVar) => {
    const item = itemsById.get(v.itemId);
    if (item?.isGlobal) {
      try {
        const owners = await getItemOwners(item.id);
        if (owners.length > 1) {
          const names = owners.map((o) => o.projectName).join(', ');
          if (!window.confirm(t('projects.confirmDeleteEverywhere', { name: ('name' in item ? item.name : (item as any).title) ?? v.key, n: owners.length, names }))) {
            return;
          }
        }
      } catch { /* fall through to delete */ }
    }
    try {
      await invoke('vault_delete_item', { id: v.itemId });
      await refreshVaultItems();
      setEnvVars((prev) => prev.filter((x) => x.itemId !== v.itemId));
      showToast(t('projects.toast.itemDeleted'));
    } catch (e) {
      reportError(e);
    }
  };

  const handleSaveEnvironment = async () => {
    if (!selectedProject) return;
    if (!isValidEnvironmentName(envName.trim())) { showToast(t('projects.invalidName', { rule: t('projects.envNameRule') }), 'error'); return; }
    setSaving(true);
    try {
      await saveEnvironment({
        id:        selectedEnv?.id,
        projectId: selectedProject.id,
        name:      envName.trim(),
        isDefault: envIsDefault,
        paths:     envPaths,
        vars:      envVars,
      });
      await refreshVaultItems();
      backToProject();
      showToast(t('projects.toast.environmentSaved'));
    } catch (e) {
      reportError(e);
    } finally {
      setSaving(false);
    }
  };

  const handleDeleteEnvironment = async () => {
    if (!selectedEnv) return;
    try {
      await removeEnvironment(selectedEnv.id);
      backToProject();
      showToast(t('projects.toast.environmentDeleted'));
    } catch (e) {
      reportError(e);
    }
  };

  const handleInjectEnvironment = async () => {
    if (!selectedEnv) return;
    if (envPaths.length === 0) { showToast(t('projects.toast.addPathFirst'), 'error'); return; }
    setInjecting(true);
    try {
      const result = await runInject(selectedEnv.id);
      const pathLabel = result.paths.length === 1 ? result.paths[0] : t('projects.toast.pathsCount', { n: result.paths.length });
      let msg = t(result.written.length === 1 ? 'projects.toast.injected_one' : 'projects.toast.injected_other', { n: result.written.length, target: pathLabel });
      if (result.unmanagedPaths.length > 0) {
        msg += t(result.unmanagedPaths.length === 1 ? 'projects.toast.unmanaged_one' : 'projects.toast.unmanaged_other', { n: result.unmanagedPaths.length });
      }
      showToast(msg);
    } catch (e) {
      if (String(e) !== 'Error: cancelled') reportError(e);
    } finally {
      setInjecting(false);
    }
  };

  // ── Render ──

  return (
    <div className="flex-1 flex flex-col overflow-hidden animate-fade-in relative">

      {mode === 'projects' && !isCreatingProj && (
        <>
          <div className="px-3.5 py-[9px] border-b border-bd flex items-center gap-[10px] shrink-0">
            <div className="flex-1 text-[13px] font-semibold text-tx">{t('projects.title')}</div>
            <button
              onClick={handleImportProject}
              className="text-[11px] font-bold font-ui text-tx2 border border-bd2 rounded-[3px] px-2.5 py-[4px] hover:text-tx transition-colors"
            >
              {t('projects.loadTemplate')}
            </button>
            <button
              onClick={() => setShareModalMode('receive')}
              className="text-[11px] font-bold font-ui text-tx2 border border-bd2 rounded-[3px] px-2.5 py-[4px] hover:text-tx transition-colors whitespace-nowrap"
            >
              {t('projects.receiveProject')}
            </button>
            <button
              onClick={handleNewProject}
              className="flex items-center gap-1 text-[11px] font-bold font-ui text-accent border border-accent-d rounded-[3px] px-2.5 py-[4px] hover:bg-accent-b transition-colors"
            >
              <Icon name="plus" size={12} />{t('projects.new')}
            </button>
          </div>

          {/* Search + tag filter */}
          <div className="flex items-center gap-3 pl-5 pr-4 h-12 border-b border-bd bg-bg shrink-0">
            <Icon name="search" size={16} />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={t('projects.searchPlaceholder')}
              className="flex-1 text-[13px] text-tx font-ui bg-transparent outline-none placeholder:text-tx3"
            />
            <div className="relative">
              <button
                onClick={() => setTagOpen((v) => !v)}
                title={t('projects.filterTitle')}
                className={[
                  'flex items-center justify-center w-6 h-6 rounded-[3px] transition',
                  tagFilter.size > 0 ? 'text-accent bg-accent-b hover:opacity-80' : 'text-tx3 hover:text-tx hover:bg-raised',
                ].join(' ')}
              >
                <Icon name="funnel" size={14} />
              </button>
              {tagOpen && (
                <div className="absolute right-0 top-8 z-50 min-w-[180px] bg-bg border border-bd rounded-[3px] shadow-lg py-1 flex flex-col">
                  <div className="px-3 py-1.5 text-[0.6rem] font-mono text-tx3 tracking-[0.1em] border-b border-bd">
                    {t('projects.filterHeader')}
                  </div>
                  {cats.length === 0 ? (
                    <div className="px-3 py-2 text-xs text-tx3 italic">{t('projects.noTags')}</div>
                  ) : (
                    cats.map((cat) => {
                      const active = tagFilter.has(cat.name);
                      return (
                        <button
                          key={cat.id}
                          onClick={() => setTagFilter((prev) => {
                            const next = new Set(prev);
                            if (next.has(cat.name)) next.delete(cat.name); else next.add(cat.name);
                            return next;
                          })}
                          className={[
                            'flex items-center gap-2 px-3 py-1.5 text-left w-full text-[12px] font-ui transition-colors duration-100',
                            active ? 'bg-raised text-tx' : 'text-tx2 hover:bg-raised hover:text-tx',
                          ].join(' ')}
                        >
                          <span className="w-2 h-2 rounded-full shrink-0" style={{ background: cat.color }} />
                          <span className="flex-1 truncate">{cat.name}</span>
                        </button>
                      );
                    })
                  )}
                  {tagFilter.size > 0 && (
                    <>
                      <div className="border-t border-bd mt-1" />
                      <button onClick={() => setTagFilter(new Set())}
                        className="flex items-center gap-2 px-3 py-1.5 text-[11px] font-ui text-tx3 hover:text-tx hover:bg-raised transition-colors w-full text-left">
                        <Icon name="close" size={11} />{t('projects.clearFilter')}
                      </button>
                    </>
                  )}
                </div>
              )}
            </div>
          </div>

          <div className="flex-1 overflow-y-auto bg-surface">
            {loading && <div className="p-4 text-[11px] text-tx3 font-mono">{t('common.loading')}</div>}
            {!loading && projects.length === 0 && (
              <div className="flex flex-col items-center justify-center h-full gap-3 text-center px-6">
                <div className="text-[11px] text-tx3 font-mono leading-[1.8]">
                  {t('projects.emptyTitle')}<br />
                  <span className="text-[10px]">{t('projects.emptyHint')}</span>
                </div>
                <button
                  onClick={handleNewProject}
                  className="flex items-center gap-1 text-[11px] font-bold font-ui text-accent border border-accent-d rounded-[3px] px-3 py-[5px] hover:bg-accent-b transition-colors"
                >
                  <Icon name="plus" size={11} />{t('projects.newProject')}
                </button>
              </div>
            )}
            {!loading && projects.length > 0 && filteredProjects.length === 0 && (
              <div className="py-16 text-center text-tx3 text-sm font-mono">{t('projects.noMatch')}</div>
            )}
            {filteredProjects.map((p) => (
              <ProjectCard key={p.id} project={p} cats={cats} onOpen={() => openProject(p)} />
            ))}
          </div>

          {/* Footer nav — Projects is now the landing screen */}
          <div className="flex items-center justify-between px-5 h-10 border-t border-bd bg-bg shrink-0">
            <div className="text-[12px] text-tx2 font-mono">{t(projects.length === 1 ? 'projects.projectCount_one' : 'projects.projectCount_other', { n: projects.length })}</div>
            <div className="flex gap-3">
              <button onClick={() => go('vault')} className="flex items-center gap-1.5 text-[13px] font-mono text-tx2 bg-transparent border-none cursor-pointer hover:text-tx transition-colors">
                <Icon name="shield" size={13} />{t('projects.navGlobalSecrets')}
              </button>
              <button onClick={() => go('categories')} className="flex items-center gap-1.5 text-[13px] font-mono text-tx2 bg-transparent border-none cursor-pointer hover:text-tx transition-colors">
                <Icon name="tag" size={13} />{t('projects.navCategories')}
              </button>
              <button onClick={() => go('settings')} className="flex items-center gap-1.5 text-[13px] font-mono text-tx2 bg-transparent border-none cursor-pointer hover:text-tx transition-colors">
                <Icon name="settings" size={13} />{t('projects.navSettings')}
              </button>
            </div>
          </div>
        </>
      )}

      {mode === 'project' && selectedProject !== null && !isCreatingProj && (
        <>
          <div className="px-3.5 py-[9px] border-b border-bd flex items-center gap-[10px] shrink-0">
            <button
              onClick={goToProjects}
              className="flex items-center gap-1 text-[12px] font-medium font-ui text-tx3 bg-transparent border-none cursor-pointer hover:text-tx transition-colors"
            >
              <Icon name="back" size={13} />{t('projects.navProjects')}
            </button>
            <div className="flex-1 text-[13px] font-semibold text-center text-tx truncate px-1">
              {selectedProject.name}
            </div>
            <button
              onClick={() => setShareModalMode('send')}
              className="text-[11px] font-bold font-ui text-tx2 border border-bd2 rounded-[3px] px-2.5 py-[4px] hover:text-tx transition-colors whitespace-nowrap"
            >
              {t('projects.shareProject')}
            </button>
            <button
              onClick={handleExportProject}
              className="text-[11px] font-bold font-ui text-tx2 border border-bd2 rounded-[3px] px-2.5 py-[4px] hover:text-tx transition-colors whitespace-nowrap"
            >
              {t('projects.saveTemplate')}
            </button>
          </div>

          <div className="flex-1 overflow-y-auto px-4 py-3">
            <div className="mb-3">
              <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.06em] mb-1">{t('projects.name')}</div>
              <input
                value={projName}
                onChange={(e) => setProjName(e.target.value)}
                placeholder="my-project"
                className="w-full bg-bg border border-bd2 text-tx font-mono text-[13px] rounded-[3px] px-3 py-[7px] outline-none focus:border-accent-d transition-colors"
              />
              {projName.length > 0 && !isValidProjectName(projName.trim()) && (
                <div className="text-[10px] font-mono text-danger mt-1">{t('projects.projectNameRule')}</div>
              )}
            </div>
            <div className="mb-3">
              <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.06em] mb-1">{t('projects.description')}</div>
              <input
                value={projDescription}
                onChange={(e) => setProjDescription(e.target.value)}
                placeholder={t('projects.descriptionPlaceholder')}
                className="w-full bg-bg border border-bd2 text-tx font-mono text-[12px] rounded-[3px] px-3 py-[7px] outline-none focus:border-accent-d transition-colors"
              />
            </div>
            <div className="mb-3">
              <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.06em] mb-1">{t('projects.tags')} <span className="text-tx3 normal-case tracking-normal font-normal">{t('projects.tagsHint')}</span></div>
              <TagInput selected={projCategories} categories={cats} onChange={setProjCategories} onCreate={handleCreateCategory} />
            </div>
            <div className="mb-4">
              <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.06em] mb-1">{t('projects.template')}</div>
              <div className="flex items-center gap-2">
                <span className="text-[11px] font-mono text-tx2 bg-raised border border-bd2 rounded-[3px] px-2 py-[3px] truncate">
                  {templateLabels(projTemplate).join(' · ')}
                </span>
                <button
                  onClick={handleSaveProject}
                  disabled={saving || !isValidProjectName(projName.trim())}
                  className="ml-auto text-[10px] font-ui font-bold text-tx2 border border-bd2 rounded-[3px] px-2.5 py-[3px] hover:text-tx transition-colors disabled:opacity-40"
                >
                  {t('common.save')}
                </button>
              </div>
            </div>

            <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.12em] mb-2 pb-1.5 border-b border-bd flex items-center justify-between">
              <span>{t('projects.environmentsHeader')}</span>
              <button
                onClick={handleNewEnvironment}
                className="flex items-center gap-1 text-accent text-[10px] font-ui font-bold hover:opacity-80 transition-opacity"
              >
                <Icon name="plus" size={10} />{t('projects.add')}
              </button>
            </div>

            {selectedProject.environments.length === 0 && (
              <div className="text-[11px] text-tx3 font-mono py-2">
                {t('projects.noEnvironments', { presets: ENV_PRESETS.map((n) => t(`projects.envPresets.${n}`)).join(', ') })}
              </div>
            )}

            {selectedProject.environments.map((env) => (
              <EnvironmentCard key={env.id} env={env} onOpen={() => openEnvironment(env)} onInject={runInject} />
            ))}
          </div>

          <div className="px-4 py-3 border-t border-bd bg-bg shrink-0">
            <div className="flex items-center gap-2">
              <div className="flex-1" />
              <button
                onClick={() => setConfirmDelProj(true)}
                className="px-3 py-[7px] rounded-[3px] text-[11px] font-bold tracking-[0.06em] font-ui cursor-pointer bg-transparent border border-bd2 text-tx3 hover:text-danger hover:border-danger transition-colors"
              >
                {t('projects.deleteProject')}
              </button>
            </div>
          </div>
        </>
      )}

      {(isCreatingProj || mode === 'environment') && (
        <>
          {/* Title is absolutely centred over the full header width so the
              back button's width can't push it off-centre; the button is
              capped (and the title padded) so the two never overlap. */}
          <div className="relative px-3.5 py-[9px] border-b border-bd flex items-center shrink-0">
            <button
              onClick={isCreatingProj ? goToProjects : backToProject}
              className="relative z-10 flex items-center gap-1 max-w-[96px] text-[12px] font-medium font-ui text-tx3 bg-transparent border-none cursor-pointer hover:text-tx transition-colors"
            >
              <Icon name="back" size={13} />
              <span className="truncate">{isCreatingProj ? t('projects.navProjects') : selectedProject?.name}</span>
            </button>
            <div className="absolute inset-0 flex items-center justify-center pointer-events-none px-[118px]">
              <span className="text-[13px] font-semibold text-tx truncate">
                {isCreatingProj ? t('projects.newProjectTitle') : (isCreatingEnv ? t('projects.newEnvironmentTitle') : (selectedEnv?.name ?? ''))}
              </span>
            </div>
          </div>

          <div className="flex-1 overflow-y-auto px-4 py-3">
            {isCreatingProj ? (
              <>
                <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.12em] mb-3 pb-1.5 border-b border-bd">
                  {t('projects.projectHeader')}
                </div>
                <div className="mb-3">
                  <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.06em] mb-1">{t('projects.name')}</div>
                  <input
                    value={projName}
                    onChange={(e) => setProjName(e.target.value)}
                    placeholder="my-project"
                    className="w-full bg-bg border border-bd2 text-tx font-mono text-[13px] rounded-[3px] px-3 py-[7px] outline-none focus:border-accent-d transition-colors"
                  />
                  {projName.length > 0 && !isValidProjectName(projName.trim()) && (
                    <div className="text-[10px] font-mono text-danger mt-1">{t('projects.projectNameRule')}</div>
                  )}
                </div>
                <div className="mb-3">
                  <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.06em] mb-1">{t('projects.description')}</div>
                  <input
                    value={projDescription}
                    onChange={(e) => setProjDescription(e.target.value)}
                    placeholder={t('projects.descriptionPlaceholder')}
                    className="w-full bg-bg border border-bd2 text-tx font-mono text-[12px] rounded-[3px] px-3 py-[7px] outline-none focus:border-accent-d transition-colors"
                  />
                </div>
                <div className="mb-3">
                  <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.06em] mb-1">{t('projects.tags')} <span className="text-tx3 normal-case tracking-normal font-normal">{t('projects.tagsHint')}</span></div>
                  <TagInput selected={projCategories} categories={cats} onChange={setProjCategories} onCreate={handleCreateCategory} />
                </div>
                <div className="mb-3">
                  <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.06em] mb-1">{t('projects.template')}</div>
                  <div className="flex items-center gap-2">
                    <span className="text-[11px] font-mono text-accent bg-accent-b border border-accent-d rounded-[3px] px-2 py-[3px] truncate">
                      {projTemplateIds.length === 0
                        ? t('projects.templateModal.generic')
                        : templateLabels(projTemplate).join(' · ')}
                    </span>
                    <button
                      onClick={() => setTemplateModal(true)}
                      className="text-[10px] font-ui font-medium text-tx2 border border-bd2 rounded-[3px] px-2 py-[3px] hover:text-tx transition-colors"
                    >
                      {t('projects.change')}
                    </button>
                  </div>
                </div>
                <div className="mb-3">
                  <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.06em] mb-1">
                    {t('projects.initialEnvironment')} <span className="text-tx3 normal-case tracking-normal font-normal">{t('projects.initialEnvironmentHint')}</span>
                  </div>
                  <input
                    value={projInitialEnv}
                    onChange={(e) => setProjInitialEnv(e.target.value)}
                    placeholder="default"
                    list="env-presets-initial"
                    className="w-full bg-bg border border-bd2 text-tx font-mono text-[12px] rounded-[3px] px-3 py-[7px] outline-none focus:border-accent-d transition-colors"
                  />
                  <EnvPresetOptions id="env-presets-initial" />
                  {!initialEnvValid && (
                    <div className="text-[10px] font-mono text-danger mt-1">{t('projects.envNameRule')}</div>
                  )}
                </div>
                <TemplateVarsReview vars={templateVars} onChange={setTemplateVars} />
                <div className="text-[11px] text-tx3 font-mono py-2">
                  {t('projects.defaultEnvNote')}
                </div>
              </>
            ) : (
              <>
                <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.12em] mb-3 pb-1.5 border-b border-bd">
                  {t('projects.environmentHeader')}
                </div>

                <div className="mb-3 flex items-center gap-2">
                  <input
                    value={envName}
                    onChange={(e) => setEnvName(e.target.value)}
                    placeholder="production"
                    list="env-presets"
                    className="flex-1 bg-bg border border-bd2 text-tx font-mono text-[13px] rounded-[3px] px-3 py-[7px] outline-none focus:border-accent-d transition-colors"
                  />
                  <EnvPresetOptions id="env-presets" />
                  <label className="flex items-center gap-1.5 text-[10px] font-mono text-tx3 shrink-0 cursor-pointer select-none">
                    <input
                      type="checkbox"
                      checked={envIsDefault}
                      onChange={(e) => setEnvIsDefault(e.target.checked)}
                      className="accent-[var(--color-accent)]"
                    />
                    {t('projects.defaultCheckbox')}
                  </label>
                </div>
                {envName.length > 0 && !isValidEnvironmentName(envName.trim()) && (
                  <div className="text-[10px] font-mono text-danger -mt-2 mb-3">{t('projects.envNameRule')}</div>
                )}

                {/* Paths */}
                <div className="mb-3">
                  <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.06em] mb-1">
                    {t('projects.paths')} <span className="text-tx3 normal-case tracking-normal font-normal">{t('projects.pathsHint')}</span>
                  </div>
                  {envPaths.map((p, idx) => (
                    <div key={idx} className="flex items-center gap-1.5 mb-1">
                      <span className="flex-1 bg-bg border border-bd2 text-tx font-mono text-[11px] rounded-[3px] px-2 py-[5px] truncate">{p}</span>
                      <button
                        onClick={() => removePath(idx)}
                        className="text-tx3 hover:text-danger transition-colors shrink-0"
                      >
                        <Icon name="trash" size={12} />
                      </button>
                    </div>
                  ))}
                  <div className="flex gap-1.5 mt-1">
                    <input
                      value={newPath}
                      onChange={(e) => setNewPath(e.target.value)}
                      onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); addPath(); } }}
                      placeholder="C:\projects\myapp\.env.production"
                      className="flex-1 bg-bg border border-bd2 text-tx font-mono text-[12px] rounded-[3px] px-3 py-[6px] outline-none focus:border-accent-d transition-colors"
                    />
                    <button
                      onClick={handlePickEnvPath}
                      title={t('projects.browseEnv')}
                      className="px-2.5 py-[6px] rounded-[3px] text-[10px] font-bold font-ui text-tx2 border border-bd2 hover:text-tx transition-colors"
                    >
                      <Icon name="external" size={12} />
                    </button>
                    {isWindows && wslDistros.length > 0 && (
                      <div className="relative">
                        <button
                          onClick={() => {
                            if (wslBusy) return;
                            if (wslDistros.length === 1) {
                              handleBrowseWsl(wslDistros[0]);
                            } else {
                              setWslPickerOpen((v) => !v);
                            }
                          }}
                          disabled={wslBusy}
                          title={t('projects.browseWsl')}
                          className="flex items-center gap-1 px-2.5 py-[6px] rounded-[3px] text-[10px] font-bold font-ui text-tx2 border border-bd2 hover:text-tx transition-colors disabled:opacity-40"
                        >
                          <Icon name="terminal" size={12} />
                          {wslBusy ? '…' : 'WSL'}
                        </button>
                        {wslPickerOpen && wslDistros.length > 1 && (
                          <div className="absolute left-0 top-8 z-50 min-w-[160px] bg-bg border border-bd rounded-[3px] shadow-lg py-1 flex flex-col">
                            {wslDistros.map((distro) => (
                              <button
                                key={distro}
                                onClick={() => handleBrowseWsl(distro)}
                                className="flex items-center gap-2 px-3 py-1.5 text-left w-full text-[11px] font-mono text-tx2 hover:bg-raised hover:text-tx transition-colors"
                              >
                                {distro}
                              </button>
                            ))}
                          </div>
                        )}
                      </div>
                    )}
                    <button
                      onClick={addPath}
                      disabled={!newPath.trim()}
                      className="px-2.5 py-[6px] rounded-[3px] text-[10px] font-bold font-ui text-accent border border-accent-d hover:bg-accent-b transition-colors disabled:opacity-40"
                    >
                      {t('projects.add')}
                    </button>
                  </div>
                </div>

                {/* Variables */}
                <div className="mt-4">
                  <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.12em] mb-2 pb-1.5 border-b border-bd flex items-center justify-between">
                    <span>{t('projects.variablesHeader')}</span>
                    {!addingVar && (
                      <button
                        onClick={() => setAddingVar(true)}
                        className="flex items-center gap-1 text-accent text-[10px] font-ui font-bold hover:opacity-80 transition-opacity"
                      >
                        <Icon name="plus" size={10} />{t('projects.add')}
                      </button>
                    )}
                  </div>

                  {addingVar && selectedProject && (
                    <AddVarPanel
                      project={selectedProject}
                      globalItems={globalItems}
                      onAdded={(v) => { setEnvVars((prev) => [...prev, v]); setAddingVar(false); }}
                      onCancel={() => setAddingVar(false)}
                    />
                  )}

                  {envVars.length === 0 && !addingVar && (
                    <div className="text-[11px] text-tx3 font-mono py-2">{t('projects.noVariables')}</div>
                  )}

                  {envVars.map((v, idx) => (
                    <VarRow
                      key={v.id}
                      v={v}
                      item={itemsById.get(v.itemId)}
                      onUnlink={() => handleUnlinkVar(idx)}
                      onDeleteEverywhere={() => handleDeleteVarEverywhere(v)}
                    />
                  ))}
                </div>
              </>
            )}
          </div>

          <div className="px-4 py-3 border-t border-bd bg-bg shrink-0">
            <div className="flex items-center gap-2">
              {!isCreatingProj && selectedEnv && (
                <button
                  onClick={handleInjectEnvironment}
                  disabled={injecting || envPaths.length === 0}
                  className="flex items-center gap-1.5 px-3 py-[7px] rounded-[3px] text-[11px] font-bold tracking-[0.06em] font-ui cursor-pointer bg-transparent border border-accent-d text-accent hover:bg-accent-b transition-colors disabled:opacity-40"
                >
                  {injecting
                    ? <><div className="w-2.5 h-2.5 rounded-full border-2 border-transparent border-t-current animate-spin-fast" />{t('projects.injecting')}</>
                    : <><Icon name="export" size={11} />{t('projects.inject')}</>}
                </button>
              )}
              <div className="flex-1" />
              {!isCreatingProj && selectedEnv && !confirmDelEnv && (
                <button
                  onClick={() => setConfirmDelEnv(true)}
                  className="px-3 py-[7px] rounded-[3px] text-[11px] font-bold tracking-[0.06em] font-ui cursor-pointer bg-transparent border border-bd2 text-tx3 hover:text-danger hover:border-danger transition-colors"
                >
                  {t('common.delete')}
                </button>
              )}
              {confirmDelEnv && (
                <>
                  <button
                    onClick={() => setConfirmDelEnv(false)}
                    className="px-3 py-[7px] rounded-[3px] text-[11px] font-ui cursor-pointer bg-transparent border border-bd2 text-tx2 hover:text-tx transition-colors"
                  >
                    {t('common.cancel')}
                  </button>
                  <button
                    onClick={handleDeleteEnvironment}
                    className="px-3 py-[7px] rounded-[3px] text-[11px] font-bold font-ui cursor-pointer bg-danger border-none text-white hover:opacity-90 transition-opacity"
                  >
                    {t('projects.confirmDelete')}
                  </button>
                </>
              )}
              <button
                onClick={isCreatingProj ? handleCreateProject : handleSaveEnvironment}
                disabled={saving || (isCreatingProj ? (!isValidProjectName(projName.trim()) || templateVarsInvalid || !initialEnvValid) : !isValidEnvironmentName(envName.trim()))}
                className="px-4 py-[7px] rounded-[3px] text-[11px] font-bold tracking-[0.06em] font-ui cursor-pointer bg-accent border-none text-[#020504] hover:opacity-90 transition-opacity disabled:opacity-40 flex items-center gap-1.5"
              >
                {saving
                  ? <><div className="w-2.5 h-2.5 rounded-full border-2 border-transparent border-t-[#020504] animate-spin-fast" />{t('common.saving')}</>
                  : t('common.save')}
              </button>
            </div>
          </div>
        </>
      )}

      {templateModal && (
        <TemplatePicker
          initial={projTemplateIds}
          onConfirm={handleTemplateSelect}
          onClose={() => {
            setTemplateModal(false);
            if (isCreatingProj && !projName) goToProjects();
          }}
        />
      )}

      {confirmDelProj && selectedProject && (
        <DeleteProjectModal
          project={selectedProject}
          onCancel={() => setConfirmDelProj(false)}
          onConfirm={handleDeleteProject}
        />
      )}

      {injectConfirm && (
        <InjectConfirmModal
          foreign={injectConfirm.foreign}
          onCancel={cancelPendingInject}
          onConfirm={confirmPendingInject}
        />
      )}

      {shareModalMode && (
        <ProjectShareModal
          mode={shareModalMode}
          project={shareModalMode === 'send' ? (selectedProject ?? undefined) : undefined}
          items={items}
          onClose={() => setShareModalMode(null)}
          onReceived={async () => {
            await load();
            await refreshVaultItems();
          }}
        />
      )}
    </div>
  );
}
