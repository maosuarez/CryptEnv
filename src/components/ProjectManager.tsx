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
  EnvironmentVar, Environment, Project, ProjectTemplate, ProjectDeleteImpact, InjectResult, VaultItem, ItemType, Category, ItemOwner,
} from '../types';
import {
  TEMPLATE_GROUPS, TEMPLATE_PLACEHOLDER, getTemplate, isValidEnvKey, mergeTemplateVars, normalizeEnvKey,
  searchTemplates, templateLabels, templateString, type MergedVar,
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

const CUSTOM_ENV = '__custom__';

/** Mirrors `is_root_environment` in `src-tauri/src/project/mod.rs`: the
 *  unnamed environment (`""`), or the legacy `"default"` name, both inject
 *  into the canonical extensionless `.env`. */
function isRootEnvName(name: string): boolean {
  return name === '' || name.toLowerCase() === 'default';
}

/** Filename an environment injects into: `.env` for the root environment,
 *  `.env.<name>` otherwise. */
function envFileName(name: string): string {
  return isRootEnvName(name) ? '.env' : `.env.${name}`;
}

/** Points a source environment's inject path at a target environment's file,
 *  when the path's filename is the source's own `.env[.<name>]` (same folder,
 *  new filename). Any other path returns `null`: it isn't obviously tied to
 *  the source environment, and copying it verbatim would make both
 *  environments inject into the same file. */
export function retargetEnvPath(path: string, fromName: string, toName: string): string | null {
  const m = path.match(/^(.*[\\/])?([^\\/]+)$/);
  if (!m || m[2] !== envFileName(fromName)) return null;
  return (m[1] ?? '') + envFileName(toName);
}

/** Whether `name` is picked from the preset dropdown (as opposed to typed
 *  into the "Custom…" field). The root `.env` counts as a preset. */
function isPresetEnvName(name: string): boolean {
  return isRootEnvName(name) || (ENV_PRESETS as readonly string[]).includes(name);
}

// Preset picker showing localized labels; "Custom…" reveals a free-text
// field. Whatever is typed is lowercased so the stored name (and the
// `.env.<name>` inject target) is always canonical lowercase ASCII. The first
// option is the unnamed root environment, shown as `.env` (empty name).
//
// `onChange` also reports whether the custom field is active, so the caller
// can tell "root .env" apart from "custom, not typed yet" (both are `""`).
// `confirmChange`, when given, gates every dropdown change (not per-keystroke
// custom typing): it receives the pending value and an `apply` callback to
// run once the user has confirmed.
function EnvNameField({
  value, onChange, confirmChange, className = '',
}: {
  value:          string;
  onChange:       (v: string, custom: boolean) => void;
  confirmChange?: (next: { value: string; custom: boolean }, apply: () => void) => void;
  className?:     string;
}) {
  const { t } = useTranslation();
  const [custom, setCustom] = useState(!isPresetEnvName(value));
  const selectValue = custom ? CUSTOM_ENV : (isRootEnvName(value) ? '' : value);
  return (
    <div className={`flex items-center gap-2 ${className}`}>
      <select
        value={selectValue}
        onChange={(e) => {
          const v = e.target.value;
          const nextCustom = v === CUSTOM_ENV;
          // Switching to custom starts from an empty field unless the name
          // was already a free-typed one.
          const nextValue = nextCustom ? (custom ? value : '') : v;
          const apply = () => { setCustom(nextCustom); onChange(nextValue, nextCustom); };
          if (confirmChange) confirmChange({ value: nextValue, custom: nextCustom }, apply);
          else apply();
        }}
        aria-label={t('projects.environmentHeader')}
        className="bg-raised border border-bd2 text-tx rounded-[3px] px-2 py-[7px] text-[12px] font-ui cursor-pointer outline-none focus:border-accent-d transition-colors"
      >
        <option value="">{t('projects.envPresets.root')}</option>
        {ENV_PRESETS.map((n) => <option key={n} value={n}>{t(`projects.envPresets.${n}`)}</option>)}
        <option value={CUSTOM_ENV}>{t('projects.envPresets.custom')}</option>
      </select>
      {custom && (
        <input
          value={value}
          onChange={(e) => onChange(e.target.value.toLowerCase(), true)}
          placeholder="my-env"
          autoFocus
          className="flex-1 min-w-0 bg-bg border border-bd2 text-tx font-mono text-[12px] rounded-[3px] px-3 py-[7px] outline-none focus:border-accent-d transition-colors"
        />
      )}
      <span className="text-[10px] font-mono text-tx3 truncate">
        {custom ? `.env.${value.trim() || '…'}` : envFileName(value)}
      </span>
    </div>
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

/** Editor-side validity: the root `.env` (empty name from the preset list) is
 *  valid; an empty *custom* name is not. */
function isValidEnvSelection(name: string, custom: boolean): boolean {
  return custom ? isValidEnvironmentName(name) : (name === '' || isValidEnvironmentName(name));
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

// ─── InjectTargetModal ──────────────────────────────────────────────────────────
// Environments with several configured paths ask where to inject first; the
// chosen subset (or `null` for all) is passed on to the preview/inject calls.

export function InjectTargetModal({
  paths,
  onCancel,
  onConfirm,
}: {
  paths:     string[];
  onCancel:  () => void;
  onConfirm: (targets: string[] | null) => void;
}) {
  const { t } = useTranslation();
  const [selected, setSelected] = useState<Set<string>>(new Set(paths));
  const all = selected.size === paths.length;

  const toggle = (p: string) => setSelected((cur) => {
    const next = new Set(cur);
    if (next.has(p)) next.delete(p); else next.add(p);
    return next;
  });

  return (
    <div className="absolute inset-0 bg-black/70 flex items-center justify-center z-30 p-4">
      <div className="w-full max-w-sm bg-surface border border-accent-d rounded-[4px] p-4">
        <div className="text-[14px] font-bold text-tx mb-3">{t('projects.injectTarget.title')}</div>
        <label className="flex items-center gap-2 text-[12px] text-tx font-ui mb-2 cursor-pointer">
          <input type="checkbox" checked={all} onChange={() => setSelected(all ? new Set() : new Set(paths))} className="accent-accent" />
          {t('projects.injectTarget.all')}
        </label>
        <ul className="mb-3 max-h-40 overflow-y-auto space-y-1 pl-5">
          {paths.map((p) => (
            <li key={p}>
              <label className="flex items-center gap-2 text-[11px] font-mono text-tx cursor-pointer">
                <input type="checkbox" checked={selected.has(p)} onChange={() => toggle(p)} className="accent-accent" />
                <span className="truncate">{p}</span>
              </label>
            </li>
          ))}
        </ul>
        <div className="flex gap-2">
          <button onClick={onCancel}
            className="flex-1 py-2 bg-transparent border border-bd2 rounded-[3px] text-tx2 text-[12px] cursor-pointer font-ui">
            {t('common.cancel')}
          </button>
          <button
            onClick={() => onConfirm(all ? null : paths.filter((p) => selected.has(p)))}
            disabled={selected.size === 0}
            className="flex-1 py-2 bg-accent border-none rounded-[3px] text-[#020504] text-[12px] font-bold cursor-pointer font-ui disabled:opacity-40"
          >
            {t('projects.injectTarget.inject')}
          </button>
        </div>
      </div>
    </div>
  );
}

// ─── ManifestExistsModal ────────────────────────────────────────────────────────
// The chosen root already holds a `.crypt-env.yaml`: overwrite it from this
// project, or keep it and just link (the root is saved either way).

function ManifestExistsModal({
  onLink,
  onOverwrite,
}: {
  onLink:      () => void;
  onOverwrite: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const [writing, setWriting] = useState(false);
  return (
    <div className="absolute inset-0 bg-black/70 flex items-center justify-center z-30 p-4">
      <div className="w-full max-w-sm bg-surface border border-accent-d rounded-[4px] p-4">
        <div className="text-[14px] font-bold text-tx mb-2">{t('projects.root.manifestExists.title')}</div>
        <div className="text-[12px] text-tx3 mb-3 leading-[1.6]">{t('projects.root.manifestExists.body')}</div>
        <div className="flex gap-2">
          <button onClick={onLink}
            className="flex-1 py-2 bg-transparent border border-bd2 rounded-[3px] text-tx2 text-[12px] cursor-pointer font-ui">
            {t('projects.root.manifestExists.link')}
          </button>
          <button
            onClick={async () => { setWriting(true); try { await onOverwrite(); } finally { setWriting(false); } }}
            disabled={writing}
            className="flex-1 py-2 bg-accent border-none rounded-[3px] text-[#020504] text-[12px] font-bold cursor-pointer font-ui disabled:opacity-40"
          >
            {t('projects.root.manifestExists.overwrite')}
          </button>
        </div>
      </div>
    </div>
  );
}

// ─── RootDirField ───────────────────────────────────────────────────────────────
// Project root directory input + folder picker (and a WSL-seeded picker on
// Windows). `.crypt-env.yaml` is written into this directory.

function RootDirField({
  value,
  error,
  onChange,
  wslDistros,
}: {
  value:      string;
  error:      string | null;
  onChange:   (v: string) => void;
  wslDistros: string[];
}) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);

  const pick = async (distro?: string) => {
    setBusy(true);
    try {
      const startDir = distro ? await invoke<string>('wsl_distro_home', { distro }) : (value || undefined);
      const picked = await invoke<string | null>('project_pick_root_dir', { startDir });
      if (picked) onChange(picked);
    } catch (e) {
      reportError(e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="mb-3">
      <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.06em] mb-1">
        {t('projects.root.label')} <span className="text-tx3 normal-case tracking-normal font-normal">{t('projects.root.hint')}</span>
      </div>
      <div className="flex gap-1.5">
        <input
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={t('projects.root.placeholder')}
          className="flex-1 bg-bg border border-bd2 text-tx font-mono text-[12px] rounded-[3px] px-3 py-[6px] outline-none focus:border-accent-d transition-colors"
        />
        <button
          onClick={() => pick()}
          disabled={busy}
          title={t('projects.root.browse')}
          className="px-2.5 py-[6px] rounded-[3px] text-[10px] font-bold font-ui text-tx2 border border-bd2 hover:text-tx transition-colors disabled:opacity-40"
        >
          <Icon name="external" size={12} />
        </button>
        {wslDistros.map((d) => (
          <button
            key={d}
            onClick={() => pick(d)}
            disabled={busy}
            title={t('projects.root.browseWsl')}
            className="px-2 py-[6px] rounded-[3px] text-[10px] font-bold font-mono text-tx2 border border-bd2 hover:text-tx transition-colors disabled:opacity-40"
          >
            {d}
          </button>
        ))}
      </div>
      {error && <div className="text-[10px] font-mono text-danger mt-1">{error}</div>}
    </div>
  );
}

/** `path` relative to `root` (with `/`) when inside it; otherwise unchanged. */
export function relativeToRoot(root: string | undefined, path: string): string {
  if (!root) return path;
  const norm = (s: string) => s.replace(/\\/g, '/');
  const r = norm(root).replace(/\/+$/, '');
  const p = norm(path);
  if (p.length > r.length + 1 && p.slice(0, r.length + 1).toLowerCase() === (r + '/').toLowerCase()) {
    return p.slice(r.length + 1);
  }
  return path;
}

function baseName(p: string): string {
  const parts = p.replace(/\\/g, '/').split('/').filter(Boolean);
  return parts[parts.length - 1] ?? '';
}

// ─── ConfirmEnvTypeChangeModal ──────────────────────────────────────────────────
// Changing a saved environment's type/preset changes its inject target file,
// so the switch only takes effect after an explicit confirmation.

function ConfirmEnvTypeChangeModal({
  from,
  to,
  onCancel,
  onConfirm,
}: {
  from:      string;
  to:        string;
  onCancel:  () => void;
  onConfirm: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="absolute inset-0 bg-black/70 flex items-center justify-center z-30 p-4">
      <div className="w-full max-w-sm bg-surface border border-accent-d rounded-[4px] p-4">
        <div className="text-[14px] font-bold text-tx mb-2">{t('projects.envTypeModal.title')}</div>
        <div className="text-[12px] text-tx3 mb-3 leading-[1.6]">{t('projects.envTypeModal.body')}</div>
        <div className="flex items-center gap-2 mb-3 text-[11px] font-mono">
          <span className="bg-bg border border-bd rounded-[2px] px-2 py-1 text-tx3 truncate">{from}</span>
          <span className="text-tx3">→</span>
          <span className="bg-accent-b border border-accent-d rounded-[2px] px-2 py-1 text-accent truncate">{to}</span>
        </div>
        <div className="flex gap-2">
          <button onClick={onCancel}
            className="flex-1 py-2 bg-transparent border border-bd2 rounded-[3px] text-tx2 text-[12px] cursor-pointer font-ui">
            {t('common.cancel')}
          </button>
          <button onClick={onConfirm}
            className="flex-1 py-2 bg-accent border-none rounded-[3px] text-[#020504] text-[12px] font-bold cursor-pointer font-ui">
            {t('projects.envTypeModal.confirm')}
          </button>
        </div>
      </div>
    </div>
  );
}

// ─── DuplicateEnvModal ──────────────────────────────────────────────────────────
// Clones an environment (vars + paths) into a new environment of a type not
// yet used in the project; a custom name is always available as a fallback.

function DuplicateEnvModal({
  source,
  usedNames,
  onCancel,
  onConfirm,
}: {
  source:    Environment;
  usedNames: string[];
  onCancel:  () => void;
  onConfirm: (name: string) => Promise<void>;
}) {
  const { t } = useTranslation();
  const rootUsed = usedNames.some(isRootEnvName);
  const freePresets = ENV_PRESETS.filter((n) => !usedNames.includes(n));
  const [target, setTarget] = useState<string>(!rootUsed ? '' : (freePresets[0] ?? CUSTOM_ENV));
  const [customName, setCustomName] = useState('');
  const [busy, setBusy] = useState(false);

  const isCustom = target === CUSTOM_ENV;
  const name = isCustom ? customName.trim() : target;
  const retargeted = source.paths.filter((p) => retargetEnvPath(p, source.name, name) !== null).length;
  const taken = isCustom && usedNames.some((n) => n.toLowerCase() === name);
  const valid = isCustom ? isValidEnvironmentName(name) && !taken : true;

  return (
    <div className="absolute inset-0 bg-black/70 flex items-center justify-center z-30 p-4">
      <div className="w-full max-w-sm bg-surface border border-accent-d rounded-[4px] p-4">
        <div className="text-[14px] font-bold text-tx mb-2">
          {t('projects.duplicateModal.title', { name: envFileName(source.name) })}
        </div>
        <div className="text-[12px] text-tx3 mb-3 leading-[1.6]">
          {t('projects.duplicateModal.body', { vars: source.vars.length, paths: retargeted, skipped: source.paths.length - retargeted })}
        </div>
        <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.06em] mb-1">{t('projects.duplicateModal.target')}</div>
        <select
          value={target}
          onChange={(e) => setTarget(e.target.value)}
          className="w-full bg-raised border border-bd2 text-tx rounded-[3px] px-2 py-[7px] text-[12px] font-ui cursor-pointer outline-none focus:border-accent-d mb-2"
        >
          {!rootUsed && <option value="">{t('projects.envPresets.root')}</option>}
          {freePresets.map((n) => <option key={n} value={n}>{t(`projects.envPresets.${n}`)}</option>)}
          <option value={CUSTOM_ENV}>{t('projects.envPresets.custom')}</option>
        </select>
        {isCustom && (
          <input
            value={customName}
            onChange={(e) => setCustomName(e.target.value.toLowerCase())}
            placeholder="my-env"
            autoFocus
            className="w-full bg-bg border border-bd2 text-tx font-mono text-[12px] rounded-[3px] px-3 py-[7px] outline-none focus:border-accent-d mb-1"
          />
        )}
        <div className="text-[10px] font-mono mb-3 min-h-[14px]">
          {isCustom && customName.length > 0 && !valid
            ? <span className="text-danger">{taken ? t('projects.duplicateModal.taken') : t('projects.envNameRule')}</span>
            : <span className="text-tx3">{isCustom ? `.env.${name || '…'}` : envFileName(name)}</span>}
        </div>
        <div className="flex gap-2">
          <button onClick={onCancel}
            className="flex-1 py-2 bg-transparent border border-bd2 rounded-[3px] text-tx2 text-[12px] cursor-pointer font-ui">
            {t('common.cancel')}
          </button>
          <button
            onClick={async () => { setBusy(true); try { await onConfirm(name); } finally { setBusy(false); } }}
            disabled={!valid || busy}
            className="flex-1 py-2 bg-accent border-none rounded-[3px] text-[#020504] text-[12px] font-bold cursor-pointer font-ui disabled:opacity-40"
          >
            {busy ? t('common.saving') : t('projects.duplicateModal.confirm')}
          </button>
        </div>
      </div>
    </div>
  );
}

// ─── ConfirmDeleteVarModal ──────────────────────────────────────────────────────
// Deleting a variable deletes its vault item everywhere, so it is always
// confirmed; owners are listed when the item is shared across projects.

function itemDisplayName(item: VaultItem): string {
  return ('name' in item ? item.name : item.title) || `#${item.id}`;
}

function ConfirmDeleteVarModal({
  varKey,
  item,
  onCancel,
  onConfirm,
}: {
  varKey:    string;
  item:      VaultItem;
  onCancel:  () => void;
  onConfirm: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const getItemOwners = useVaultStore((s) => s.getItemOwners);
  const [owners, setOwners] = useState<ItemOwner[] | null>(null);
  const [deleting, setDeleting] = useState(false);

  useEffect(() => {
    getItemOwners(item.id).then(setOwners).catch(() => setOwners(null));
  }, [item.id]);

  return (
    <div className="absolute inset-0 bg-black/70 flex items-center justify-center z-30 p-4">
      <div className="w-full max-w-sm bg-surface border border-danger rounded-[4px] p-4">
        <div className="text-[14px] font-bold text-tx mb-2">{t('projects.deleteVarModal.title', { key: varKey })}</div>
        <div className="text-[12px] text-tx3 mb-3 leading-[1.6]">
          {t('projects.deleteVarModal.body', { name: itemDisplayName(item) })}
        </div>
        {owners && owners.length > 1 && (
          <div className="text-[11px] text-danger font-mono mb-3 leading-[1.6]">
            {t('projects.deleteVarModal.shared', { n: owners.length, names: owners.map((o) => o.projectName).join(', ') })}
          </div>
        )}
        <div className="flex gap-2">
          <button onClick={onCancel}
            className="flex-1 py-2 bg-transparent border border-bd2 rounded-[3px] text-tx2 text-[12px] cursor-pointer font-ui">
            {t('common.cancel')}
          </button>
          <button
            onClick={async () => { setDeleting(true); try { await onConfirm(); } finally { setDeleting(false); } }}
            disabled={deleting}
            className="flex-1 py-2 bg-danger border-none rounded-[3px] text-white text-[12px] font-bold cursor-pointer font-ui disabled:opacity-40"
          >
            {deleting ? t('projects.deleteModal.deleting') : t('common.delete')}
          </button>
        </div>
      </div>
    </div>
  );
}

// ─── VarDetailModal ─────────────────────────────────────────────────────────────
// Full view of the vault item behind an environment variable. Values come
// from the (already unlocked) vault store and stay masked until revealed; the
// reveal state lives in this component and is discarded on close.

function VarDetailModal({
  varKey,
  item,
  onClose,
  onToggleGlobal,
}: {
  varKey:         string;
  item:           VaultItem;
  onClose:        () => void;
  onToggleGlobal: (global: boolean) => Promise<void>;
}) {
  const { t } = useTranslation();
  const getItemOwners = useVaultStore((s) => s.getItemOwners);
  const [owners, setOwners] = useState<ItemOwner[] | null>(null);
  const [revealed, setRevealed] = useState<Set<string>>(new Set());
  const [toggling, setToggling] = useState(false);

  useEffect(() => {
    getItemOwners(item.id).then(setOwners).catch(() => setOwners(null));
  }, [item.id, item.isGlobal]);

  type FieldLabel = 'name' | 'value' | 'url' | 'username' | 'password' | 'title' | 'description' | 'command' | 'shell' | 'content';
  const fields: { label: FieldLabel; value?: string; sensitive?: boolean }[] = (() => {
    switch (item.type) {
      case 'secret':     return [{ label: 'name', value: item.name }, { label: 'value', value: item.value, sensitive: true }];
      case 'credential': return [{ label: 'name', value: item.name }, { label: 'url', value: item.url }, { label: 'username', value: item.username }, { label: 'password', value: item.password, sensitive: true }];
      case 'link':       return [{ label: 'title', value: item.title }, { label: 'url', value: item.url }, { label: 'description', value: item.description }];
      case 'command':    return [{ label: 'name', value: item.name }, { label: 'command', value: item.command }, { label: 'shell', value: item.shell }, { label: 'description', value: item.description }];
      case 'note':       return [{ label: 'title', value: item.title }, { label: 'content', value: item.content }];
    }
  })();
  const toggleReveal = (label: string) =>
    setRevealed((r) => { const n = new Set(r); if (n.has(label)) n.delete(label); else n.add(label); return n; });

  return (
    <div
      className="absolute inset-0 bg-black/70 flex items-center justify-center z-30 p-4"
      onKeyDown={(e) => { if (e.key === 'Escape') onClose(); }}
    >
      <div className="w-full max-w-md max-h-full overflow-y-auto bg-surface border border-bd rounded-[4px] p-4">
        <div className="flex items-center gap-2 mb-3">
          <span className="flex-1 font-mono text-[14px] font-bold text-tx truncate">{varKey}</span>
          <span className="text-[9px] font-mono text-tx3 bg-bg border border-bd px-1.5 py-[1px] rounded-[2px] uppercase shrink-0">
            {t(`projects.varGroups.${item.type}`)}
          </span>
          <button onClick={onClose} aria-label={t('common.close')} className="text-tx3 hover:text-tx transition-colors">
            <Icon name="close" size={13} />
          </button>
        </div>

        {fields.filter((f) => f.value).map((f) => {
          const masked = f.sensitive && !revealed.has(f.label);
          return (
            <div key={f.label} className="mb-2">
              <div className="text-[9px] font-semibold text-tx3 font-mono tracking-[0.1em] uppercase mb-0.5">{t(`projects.varDetail.fields.${f.label}`)}</div>
              <div className="flex items-start gap-2">
                <div className="flex-1 min-w-0 bg-bg border border-bd2 text-tx font-mono text-[11px] rounded-[3px] px-2 py-[5px] whitespace-pre-wrap break-all">
                  {masked ? '••••••••' : f.value}
                </div>
                {f.sensitive && (
                  <button onClick={() => toggleReveal(f.label)} className="text-tx3 hover:text-tx transition-colors shrink-0 mt-1">
                    <Icon name={masked ? 'eye' : 'eyeOff'} size={12} />
                  </button>
                )}
              </div>
            </div>
          );
        })}

        {item.notes && (
          <div className="mb-2">
            <div className="text-[9px] font-semibold text-tx3 font-mono tracking-[0.1em] uppercase mb-0.5">{t('projects.varDetail.fields.notes')}</div>
            <div className="bg-bg border border-bd2 text-tx font-mono text-[11px] rounded-[3px] px-2 py-[5px] whitespace-pre-wrap break-all">{item.notes}</div>
          </div>
        )}

        <div className="grid grid-cols-2 gap-2 mb-3 text-[10px] font-mono">
          <div>
            <div className="text-tx3 tracking-[0.1em] uppercase mb-0.5">{t('projects.varDetail.created')}</div>
            <div className="text-tx2">{item.created}</div>
          </div>
          <div>
            <div className="text-tx3 tracking-[0.1em] uppercase mb-0.5">{t('projects.varDetail.categories')}</div>
            <div className="text-tx2 truncate">{item.categories.length > 0 ? item.categories.join(', ') : '—'}</div>
          </div>
        </div>

        <div className="mb-3">
          <div className="text-[9px] font-semibold text-tx3 font-mono tracking-[0.1em] uppercase mb-0.5">{t('projects.varDetail.owners')}</div>
          <div className="text-[11px] font-mono text-tx2">
            {owners === null ? '…' : owners.length === 0 ? '—' : owners.map((o) => o.projectName).join(', ')}
          </div>
        </div>

        <label className="flex items-center gap-2 border-t border-bd pt-3 text-[11px] font-ui text-tx cursor-pointer select-none">
          <input
            type="checkbox"
            checked={!!item.isGlobal}
            disabled={toggling}
            onChange={async (e) => {
              setToggling(true);
              try { await onToggleGlobal(e.target.checked); } finally { setToggling(false); }
            }}
            className="accent-[var(--color-accent)]"
          />
          <span className="flex-1">{t('projects.varDetail.global')}</span>
        </label>
        <div className="text-[10px] font-mono text-tx3 mt-1">{t('projects.varDetail.globalHint')}</div>
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
  onOpen,
  onDelete,
  onUnlink,
  onRemoveMissing,
}: {
  v:               EnvironmentVar;
  item:            VaultItem | undefined;
  onOpen:          () => void;
  onDelete:        () => void;
  onUnlink:        () => void;
  onRemoveMissing: () => void;
}) {
  const { t } = useTranslation();
  const [reveal, setReveal] = useState(false);

  if (!item) {
    // Dangling link to an item that no longer exists — nothing to delete in
    // the vault, so the trash only drops the reference.
    return (
      <div className="py-2 border-b border-bd flex items-center gap-2">
        <span className="flex-1 text-[11px] font-mono text-danger truncate">{v.key} — {t('projects.varRow.itemMissing')}</span>
        <button onClick={onRemoveMissing} className="text-tx3 hover:text-danger transition-colors shrink-0">
          <Icon name="trash" size={13} />
        </button>
      </div>
    );
  }

  return (
    <div className="py-2 border-b border-bd group">
      <div className="flex items-center gap-2 mb-1">
        <button onClick={onOpen} title={t('projects.varRow.details')}
          className="flex-1 min-w-0 text-left font-mono text-[12px] text-tx truncate hover:text-accent transition-colors">
          {v.key}
        </button>
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
        {/* Global items are shared, so trash only unlinks them from this
            environment; they're deleted from the Global Secrets view. */}
        <button onClick={item.isGlobal ? onUnlink : onDelete}
          title={t(item.isGlobal ? 'projects.varRow.unlinkGlobal' : 'projects.varRow.deleteEverywhere')}
          className="text-tx3 hover:text-danger transition-colors shrink-0 opacity-0 group-hover:opacity-100">
          <Icon name="trash" size={12} />
        </button>
      </div>
    </div>
  );
}

// ─── VarGroups (environment variables, accordion per item type) ───────────────

const VAR_GROUP_ORDER = ['secret', 'credential', 'link', 'command', 'note', 'missing'] as const;
type VarGroup = typeof VAR_GROUP_ORDER[number];

function VarGroups({
  vars,
  itemsById,
  collapsed,
  onToggle,
  renderRow,
}: {
  vars:      EnvironmentVar[];
  itemsById: Map<number, VaultItem>;
  collapsed: Set<VarGroup>;
  onToggle:  (g: VarGroup) => void;
  renderRow: (v: EnvironmentVar, idx: number) => React.ReactNode;
}) {
  const { t } = useTranslation();
  const groups = new Map<VarGroup, { v: EnvironmentVar; idx: number }[]>();
  vars.forEach((v, idx) => {
    const g: VarGroup = itemsById.get(v.itemId)?.type ?? 'missing';
    const list = groups.get(g);
    if (list) list.push({ v, idx }); else groups.set(g, [{ v, idx }]);
  });

  return (
    <>
      {VAR_GROUP_ORDER.filter((g) => groups.has(g)).map((g) => {
        const rows = groups.get(g) ?? [];
        const open = !collapsed.has(g);
        return (
          <div key={g} className="mb-2">
            <button
              onClick={() => onToggle(g)}
              aria-expanded={open}
              className="w-full flex items-center gap-1.5 py-1.5 text-left text-[10px] font-semibold font-mono tracking-[0.08em] uppercase text-tx2 hover:text-tx transition-colors"
            >
              <span className={`inline-block w-2.5 text-center transition-transform ${open ? 'rotate-90' : ''}`}>▸</span>
              <span className="flex-1">{t(`projects.varGroups.${g}`)}</span>
              <span className="text-tx3">({rows.length})</span>
            </button>
            {open && <div className="pl-3">{rows.map(({ v, idx }) => renderRow(v, idx))}</div>}
          </div>
        );
      })}
    </>
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
  const toggleGlobal     = useVaultStore((s) => s.toggleGlobal);

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
  const [isGlobal, setIsGlobal] = useState(false);

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
      // `vault_create_project_item` always creates a project-local item (a
      // backend invariant), so "global" is a separate, explicit flip — the
      // item has a single owner here, so this never forks.
      if (isGlobal) await toggleGlobal(item.id, true);
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
          <label className="flex items-center gap-2 mb-2 text-[11px] font-ui text-tx2 cursor-pointer select-none">
            <input
              type="checkbox"
              checked={isGlobal}
              onChange={(e) => setIsGlobal(e.target.checked)}
              className="accent-[var(--color-accent)]"
            />
            {t('projects.addVar.markGlobal')}
          </label>
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
  const envNames = project.environments.map((e) => envFileName(e.name));
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
  isDefault,
  onOpen,
  onInject,
}: {
  env:       Environment;
  isDefault: boolean;
  onOpen:    () => void;
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
        <span className="flex-1 text-[13px] font-semibold text-tx font-ui font-mono truncate">{envFileName(env.name)}</span>
        {isDefault && (
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
  const toggleGlobal   = useVaultStore((s) => s.toggleGlobal);
  const saveCats       = useVaultStore((s) => s.saveCats);

  const { projects, loading, load, saveProject, createFromTemplates, removeProject, saveEnvironment, removeEnvironment, inject, previewInject, createProjectItem } =
    useProjectStore();

  // Pending confirm-then-inject flow (see `runInject` below): `injectConfirm`
  // holds the modal's display data, while the resolve/reject pair for the
  // promise `runInject` returned to its caller lives in a ref so it survives
  // re-renders without becoming React state itself.
  const [injectConfirm, setInjectConfirm] = useState<{ id: number; foreign: string[]; targets?: string[] } | null>(null);
  // Multi-path environments pick their inject target(s) first.
  const [injectTargets, setInjectTargets] = useState<{ paths: string[] } | null>(null);
  const pendingTargetsRef = useRef<{ resolve: (t: string[] | null) => void; reject: (e: unknown) => void } | null>(null);
  const pendingInjectRef = useRef<{ resolve: (r: InjectResult) => void; reject: (e: unknown) => void } | null>(null);

  // Single entry point for both the project-list quick-inject button and the
  // environment editor's INJECT button: previews first, and only prompts for
  // confirmation when the preview reports a path crypt-env doesn't manage.
  const runInject = async (id: number): Promise<InjectResult> => {
    const env = projects.flatMap((p) => p.environments).find((e) => e.id === id);
    let targets: string[] | undefined;
    if (env && env.paths.length > 1) {
      const picked = await new Promise<string[] | null>((resolve, reject) => {
        pendingTargetsRef.current = { resolve, reject };
        setInjectTargets({ paths: env.paths });
      });
      targets = picked ?? undefined;
    }
    const preview = await previewInject(id, targets);
    if (preview.symlinks.length > 0) {
      throw new Error(`Refusing to write through a symlink: ${preview.symlinks.join(', ')}. Point the environment at the real file instead.`);
    }
    if (preview.foreign.length === 0) return inject(id, false, targets);
    return new Promise<InjectResult>((resolve, reject) => {
      pendingInjectRef.current = { resolve, reject };
      setInjectConfirm({ id, foreign: preview.foreign, targets });
    });
  };

  const settleInjectTargets = (targets: string[] | null | 'cancel') => {
    const pending = pendingTargetsRef.current;
    pendingTargetsRef.current = null;
    setInjectTargets(null);
    if (targets === 'cancel') pending?.reject(new Error('cancelled'));
    else pending?.resolve(targets);
  };

  const confirmPendingInject = async () => {
    if (!injectConfirm) return;
    const { id, targets } = injectConfirm;
    const pending = pendingInjectRef.current;
    pendingInjectRef.current = null;
    setInjectConfirm(null);
    try {
      const result = await inject(id, true, targets);
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
  const [projInitialCustom, setProjInitialCustom] = useState(false);
  const [templateVars,    setTemplateVars]    = useState<ReviewVar[]>([]);
  const [projCategories,  setProjCategories]  = useState<string[]>([]);
  const [projRoot,        setProjRoot]        = useState('');
  const [rootError,       setRootError]       = useState<string | null>(null);
  const [manifestPrompt,  setManifestPrompt]  = useState<number | null>(null);
  const [isCreatingProj,  setIsCreatingProj]  = useState(false);
  const [templateModal,   setTemplateModal]   = useState(false);
  const [confirmDelProj,  setConfirmDelProj]  = useState(false);
  const [shareModalMode,  setShareModalMode]  = useState<ProjectShareMode | null>(null);

  // Environment form state
  const [envName,       setEnvName]       = useState('');
  const [envCustom,     setEnvCustom]     = useState(false);
  const [envIsDefault,  setEnvIsDefault]  = useState(false);
  const [envPaths,      setEnvPaths]      = useState<string[]>([]);
  const [newPath,       setNewPath]       = useState('');
  const [envVars,       setEnvVars]       = useState<EnvironmentVar[]>([]);
  const [isCreatingEnv, setIsCreatingEnv] = useState(false);
  const [confirmDelEnv, setConfirmDelEnv] = useState(false);
  const [addingVar,     setAddingVar]     = useState(false);
  // Template variables offered to a new environment of a templated project
  // (created as project items on save).
  const [envTemplateVars, setEnvTemplateVars] = useState<ReviewVar[]>([]);
  // Accordion state survives switching environments for the view session.
  const [collapsedGroups, setCollapsedGroups] = useState<Set<VarGroup>>(new Set());
  const [pendingTypeChange, setPendingTypeChange] = useState<{ to: string; apply: () => void } | null>(null);
  const [duplicating,   setDuplicating]   = useState(false);
  const [deleteVarTarget, setDeleteVarTarget] = useState<EnvironmentVar | null>(null);
  const [detailVar,     setDetailVar]     = useState<EnvironmentVar | null>(null);

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
    setProjRoot(p.rootPath ?? '');
    setRootError(null);
    setIsCreatingProj(false);
    setConfirmDelProj(false);
    setMode('project');
  };

  const openEnvironment = (env: Environment) => {
    setSelectedEnv(env);
    // Legacy "default" is the root `.env`; it's shown (and saved) unnamed.
    setEnvName(isRootEnvName(env.name) ? '' : env.name);
    setEnvCustom(!isPresetEnvName(env.name));
    setEnvTemplateVars([]);
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
    setProjInitialCustom(false);
    setProjCategories([]);
    setProjRoot('');
    setRootError(null);
    setIsCreatingProj(true);
    setConfirmDelProj(false);
    setTemplateModal(true);
  };

  // Re-picking templates rebuilds the variable list from scratch (edits are
  // discarded). Templates seed variables only — categories stay user-curated,
  // so picking templates never creates vault categories.
  const handleTemplateSelect = (ids: string[]) => {
    setProjTemplateIds(ids);
    setProjTemplate(templateString(ids));
    setTemplateVars(toReviewVars(mergeTemplateVars(ids)));
    setTemplateModal(false);
    setMode('project');
  };

  // Root `.env` (blank) is fine; a custom name must follow the env-name rule.
  const initialEnvValid = isValidEnvSelection(projInitialEnv.trim(), projInitialCustom);

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

  // Inline validation of the root field; `true` when usable.
  const validateRoot = async (root: string): Promise<boolean> => {
    if (!root) { setRootError(t('projects.root.required')); return false; }
    const check = await invoke<{ problem: 'notAbsolute' | 'notFound' | 'notDir' | 'notWritable' | null; hasManifest: boolean }>('project_check_root', { root });
    if (check.problem) { setRootError(t(`projects.root.problems.${check.problem}`)); return false; }
    setRootError(null);
    return true;
  };

  // Writes `.crypt-env.yaml` into the project root; an existing file opens
  // the overwrite / link-only prompt instead.
  const writeManifest = async (projectId: number, overwrite = false) => {
    try {
      const path = await invoke<string>('project_write_yaml', { projectId, overwrite });
      showToast(t('projects.root.yamlWritten', { path }));
    } catch (e) {
      if (String(e) === 'manifest exists') setManifestPrompt(projectId);
      else reportError(e);
    }
  };

  const handleRootChange = (v: string) => {
    setProjRoot(v);
    setRootError(null);
    if (isCreatingProj && !projName) {
      const name = baseName(v);
      if (name) setProjName(name);
    }
  };

  const handleCreateProject = async () => {
    if (!isValidProjectName(projName.trim())) { showToast(t('projects.invalidName', { rule: t('projects.projectNameRule') }), 'error'); return; }
    if (templateVarsInvalid || !initialEnvValid) return;
    const root = projRoot.trim();
    setSaving(true);
    try {
      if (!(await validateRoot(root))) return;
      await ensureCategories(projCategories);
      const id = await createFromTemplates({
        name:        projName.trim(),
        description: projDescription || undefined,
        template:    templateString(projTemplateIds),
        categories:  projCategories,
        initialEnvironment: projInitialEnv.trim().toLowerCase() || undefined,
        rootPath:    root,
        vars:        templateVars.map((v) => ({ key: v.key, value: v.value })),
      });
      await refreshVaultItems();
      await writeManifest(id);
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

  // Save is only offered while name / description / categories differ from
  // the persisted project.
  const projectDirty = !!selectedProject && !isCreatingProj && (
    projName.trim() !== selectedProject.name ||
    projDescription.trim() !== (selectedProject.description ?? '') ||
    projCategories.length !== selectedProject.categories.length ||
    projCategories.some((c) => !selectedProject.categories.includes(c)) ||
    projRoot.trim() !== (selectedProject.rootPath ?? '')
  );

  const handleSaveProject = async () => {
    if (!isValidProjectName(projName.trim())) { showToast(t('projects.invalidName', { rule: t('projects.projectNameRule') }), 'error'); return; }
    const root = projRoot.trim();
    const rootChanged = root !== (selectedProject?.rootPath ?? '');
    setSaving(true);
    try {
      // Clearing the root is allowed; a new root must be usable.
      if (rootChanged && root && !(await validateRoot(root))) return;
      const id = await saveProject({
        id:          selectedProject?.id,
        name:        projName.trim(),
        description: projDescription || undefined,
        template:    projTemplate,
        categories:  projCategories,
        rootPath:    root,
      });
      if (rootChanged && root) await writeManifest(id);
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
    // Suggest the root `.env` first, then the first unused preset.
    const used = selectedProject.environments.map((e) => e.name);
    const firstFree = used.some(isRootEnvName) ? (ENV_PRESETS.find((n) => !used.includes(n)) ?? '') : '';
    setEnvName(firstFree);
    setEnvCustom(false);
    setEnvIsDefault(selectedProject.environments.length === 0);
    setEnvPaths([]);
    setNewPath('');
    setEnvVars([]);
    // Keep the project's stack templates: offer their variables for review.
    const templateIds = selectedProject.template.split(',').map((id) => id.trim()).filter(Boolean);
    setEnvTemplateVars(toReviewVars(mergeTemplateVars(templateIds)));
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
    const trimmed = relativeToRoot(selectedProject?.rootPath, picked.trim());
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

  // Drops a var from this environment only (dangling reference, or unlinking
  // a global item); the vault item itself is untouched. Takes effect on save.
  const handleUnlinkVar = (idx: number) => {
    setEnvVars((prev) => prev.filter((_, i) => i !== idx));
  };

  const handleDeleteVar = async (v: EnvironmentVar) => {
    try {
      await invoke('vault_delete_item', { id: v.itemId });
      await refreshVaultItems();
      setEnvVars((prev) => prev.filter((x) => x.itemId !== v.itemId));
      setDeleteVarTarget(null);
      showToast(t('projects.toast.itemDeleted'));
    } catch (e) {
      reportError(e);
    }
  };

  const handleToggleVarGlobal = async (v: EnvironmentVar, global: boolean) => {
    try {
      const result = await toggleGlobal(v.itemId, global);
      if (result.forked.length > 0) {
        // Un-globaling a shared item forks it and repoints each project's
        // saved vars at its own copy — follow that repoint in the editor.
        await load();
        const env = useProjectStore.getState().projects
          .find((p) => p.id === selectedProject?.id)?.environments
          .find((e) => e.id === selectedEnv?.id);
        const fork = env?.vars.find((x) => x.key === v.key && x.itemId !== v.itemId);
        if (fork) {
          setEnvVars((prev) => prev.map((x) => (x.itemId === v.itemId ? { ...x, itemId: fork.itemId } : x)));
          setDetailVar({ ...v, itemId: fork.itemId });
        } else {
          setDetailVar(null);
        }
      }
      showToast(t(global ? 'projects.toast.markedGlobal' : 'projects.toast.unmarkedGlobal'));
    } catch (e) {
      reportError(e);
    }
  };

  // Default rules: an unnamed root `.env` is always the project default;
  // only when none exists can another environment be marked default.
  const editingRoot = !envCustom && envName === '';
  const otherRootExists = !!selectedProject?.environments.some((e) => e.id !== selectedEnv?.id && isRootEnvName(e.name));
  const effectiveIsDefault = editingRoot ? true : otherRootExists ? false : envIsDefault;
  const defaultLocked = editingRoot || otherRootExists;

  const envTemplateVarsInvalid = useMemo(() => invalidReviewKeys(envTemplateVars).size > 0, [envTemplateVars]);

  const handleEnvNameChange = (v: string, custom: boolean) => {
    setEnvName(v);
    setEnvCustom(custom);
  };

  // Only a saved environment's type change needs confirming: its inject
  // target on disk is what changes.
  const confirmEnvTypeChange = selectedEnv
    ? (next: { value: string; custom: boolean }, apply: () => void) => {
        setPendingTypeChange({ to: next.custom ? `.env.${next.value || '…'}` : envFileName(next.value), apply });
      }
    : undefined;

  const handleDuplicateEnvironment = async (name: string) => {
    if (!selectedProject || !selectedEnv) return;
    const hasOtherDefault = selectedProject.environments.some((e) => e.isDefault || isRootEnvName(e.name));
    try {
      const id = await saveEnvironment({
        projectId: selectedProject.id,
        name,
        isDefault: name === '' || !hasOtherDefault,
        paths:     selectedEnv.paths
          .map((p) => retargetEnvPath(p, selectedEnv.name, name))
          .filter((p): p is string => p !== null),
        vars:      selectedEnv.vars.map((v) => ({ ...v, id: 0 })),
      });
      setDuplicating(false);
      const fresh = useProjectStore.getState().projects
        .find((p) => p.id === selectedProject.id)?.environments.find((e) => e.id === id);
      if (fresh) openEnvironment(fresh);
      showToast(t('projects.toast.environmentDuplicated'));
    } catch (e) {
      reportError(e);
    }
  };

  const handleSaveEnvironment = async () => {
    if (!selectedProject) return;
    if (!isValidEnvSelection(envName.trim(), envCustom)) { showToast(t('projects.invalidName', { rule: t('projects.envNameRule') }), 'error'); return; }
    if (envTemplateVarsInvalid) return;
    // Pre-check the name so template items are never created for a save
    // that the backend would then reject as a collision.
    const lower = envName.trim().toLowerCase();
    const collides = selectedProject.environments.some((e) => e.id !== selectedEnv?.id
      && (e.name.toLowerCase() === lower || (lower === '' && isRootEnvName(e.name)) || (isRootEnvName(lower) && isRootEnvName(e.name))));
    if (collides) { showToast(t('projects.duplicateModal.taken'), 'error'); return; }
    setSaving(true);
    try {
      // Template variables become project-local secrets, like project
      // scaffolding does; an empty value is stored as the placeholder.
      const templateLinks: EnvironmentVar[] = [];
      for (const tv of envTemplateVars) {
        if (envVars.some((v) => v.key === tv.key)) continue;
        const item = await createProjectItem(selectedProject.id, {
          type: 'secret', name: tv.key, value: tv.value || TEMPLATE_PLACEHOLDER, categories: [], isGlobal: false,
        } as Omit<VaultItem, 'id' | 'created'>);
        templateLinks.push({ id: 0, key: tv.key, itemId: item.id });
      }
      await saveEnvironment({
        id:        selectedEnv?.id,
        projectId: selectedProject.id,
        name:      envName.trim().toLowerCase(),
        isDefault: effectiveIsDefault,
        paths:     envPaths,
        vars:      [...envVars, ...templateLinks],
      });
      setEnvTemplateVars([]);
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
                    <div className="px-3 py-2 text-xs text-tx3 italic">{t('projects.noCategories')}</div>
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
            <RootDirField value={projRoot} error={rootError} onChange={handleRootChange} wslDistros={isWindows ? wslDistros : []} />
            <div className="mb-3">
              <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.06em] mb-1">{t('projects.categories')} <span className="text-tx3 normal-case tracking-normal font-normal">{t('projects.categoriesHint')}</span></div>
              <TagInput selected={projCategories} categories={cats} onChange={setProjCategories} onCreate={handleCreateCategory} />
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

            {(() => {
              // An unnamed root `.env` is the default whenever it exists.
              const hasRoot = selectedProject.environments.some((e) => isRootEnvName(e.name));
              return selectedProject.environments.map((env) => (
                <EnvironmentCard
                  key={env.id}
                  env={env}
                  isDefault={hasRoot ? isRootEnvName(env.name) : env.isDefault}
                  onOpen={() => openEnvironment(env)}
                  onInject={runInject}
                />
              ));
            })()}
          </div>

          <div className="px-4 py-3 border-t border-bd bg-bg shrink-0">
            <div className="flex items-center gap-2">
              <div className="flex-1" />
              {projectDirty && (
                <button
                  onClick={handleSaveProject}
                  disabled={saving || !isValidProjectName(projName.trim())}
                  className="px-3 py-[7px] rounded-[3px] text-[11px] font-bold tracking-[0.06em] font-ui cursor-pointer bg-accent text-[#020504] border-none hover:opacity-90 transition-opacity disabled:opacity-40"
                >
                  {t('common.save')}
                </button>
              )}
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
                {isCreatingProj ? t('projects.newProjectTitle') : (isCreatingEnv ? t('projects.newEnvironmentTitle') : (selectedEnv ? envFileName(selectedEnv.name) : ''))}
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
                <RootDirField value={projRoot} error={rootError} onChange={handleRootChange} wslDistros={isWindows ? wslDistros : []} />
                <div className="mb-3">
                  <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.06em] mb-1">{t('projects.categories')} <span className="text-tx3 normal-case tracking-normal font-normal">{t('projects.categoriesHint')}</span></div>
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
                  <EnvNameField value={projInitialEnv} onChange={(v, custom) => { setProjInitialEnv(v); setProjInitialCustom(custom); }} />
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
                  <EnvNameField
                    key={selectedEnv?.id ?? 'new'}
                    value={envName}
                    onChange={handleEnvNameChange}
                    confirmChange={confirmEnvTypeChange}
                    className="flex-1 min-w-0"
                  />
                  <label
                    title={defaultLocked ? t(editingRoot ? 'projects.defaultLockedRoot' : 'projects.defaultLockedOther') : undefined}
                    className={`flex items-center gap-1.5 text-[10px] font-mono text-tx3 shrink-0 select-none ${defaultLocked ? 'opacity-60 cursor-not-allowed' : 'cursor-pointer'}`}
                  >
                    <input
                      type="checkbox"
                      checked={effectiveIsDefault}
                      disabled={defaultLocked}
                      onChange={(e) => setEnvIsDefault(e.target.checked)}
                      className="accent-[var(--color-accent)]"
                    />
                    {t('projects.defaultCheckbox')}
                  </label>
                  {selectedEnv && (
                    <button
                      onClick={() => setDuplicating(true)}
                      title={t('projects.duplicateEnv')}
                      className="shrink-0 px-2 py-[5px] rounded-[3px] text-[10px] font-bold font-ui text-tx2 border border-bd2 hover:text-tx transition-colors"
                    >
                      <Icon name="copy" size={12} />
                    </button>
                  )}
                </div>
                {envCustom && envName.length > 0 && !isValidEnvironmentName(envName.trim()) && (
                  <div className="text-[10px] font-mono text-danger -mt-2 mb-3">{t('projects.envNameRule')}</div>
                )}

                {/* Paths */}
                <div className="mb-3">
                  <div className="text-[10px] font-semibold text-tx3 font-mono tracking-[0.06em] mb-1">
                    {t('projects.paths')} <span className="text-tx3 normal-case tracking-normal font-normal">
                      {selectedProject?.rootPath ? t('projects.pathsRelativeHint', { root: selectedProject.rootPath }) : t('projects.pathsHint')}
                    </span>
                  </div>
                  {envPaths.map((p, idx) => (
                    <div key={idx} className="flex items-center gap-1.5 mb-1">
                      <input
                        value={p}
                        onChange={(e) => { const v = e.target.value; setEnvPaths((prev) => prev.map((x, i) => (i === idx ? v : x))); }}
                        onBlur={() => setEnvPaths((prev) => prev.map((x) => x.trim()).filter((x, i, a) => x && a.indexOf(x) === i))}
                        className="flex-1 min-w-0 bg-bg border border-bd2 text-tx font-mono text-[11px] rounded-[3px] px-2 py-[5px] outline-none focus:border-accent-d transition-colors"
                      />
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
                      placeholder={selectedProject?.rootPath ? 'apps/api/.env.production' : 'C:\\projects\\myapp\\.env.production'}
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

                  {isCreatingEnv && envTemplateVars.length > 0 && (
                    <>
                      <div className="text-[10px] font-mono text-tx3 mb-1">
                        {t('projects.envTemplateHint', { templates: templateLabels(selectedProject?.template ?? '').join(' · ') })}
                      </div>
                      <TemplateVarsReview vars={envTemplateVars} onChange={setEnvTemplateVars} />
                    </>
                  )}

                  {envVars.length === 0 && !addingVar && envTemplateVars.length === 0 && (
                    <div className="text-[11px] text-tx3 font-mono py-2">{t('projects.noVariables')}</div>
                  )}

                  <VarGroups
                    vars={envVars}
                    itemsById={itemsById}
                    collapsed={collapsedGroups}
                    onToggle={(g) => setCollapsedGroups((cur) => {
                      const next = new Set(cur);
                      if (next.has(g)) next.delete(g); else next.add(g);
                      return next;
                    })}
                    renderRow={(v, idx) => (
                      <VarRow
                        key={v.id}
                        v={v}
                        item={itemsById.get(v.itemId)}
                        onOpen={() => setDetailVar(v)}
                        onDelete={() => setDeleteVarTarget(v)}
                        onUnlink={() => handleUnlinkVar(idx)}
                        onRemoveMissing={() => handleUnlinkVar(idx)}
                      />
                    )}
                  />
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
                disabled={saving || (isCreatingProj ? (!isValidProjectName(projName.trim()) || templateVarsInvalid || !initialEnvValid) : (!isValidEnvSelection(envName.trim(), envCustom) || envTemplateVarsInvalid))}
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

      {pendingTypeChange && selectedEnv && (
        <ConfirmEnvTypeChangeModal
          from={envCustom ? `.env.${envName || '…'}` : envFileName(envName)}
          to={pendingTypeChange.to}
          onCancel={() => setPendingTypeChange(null)}
          onConfirm={() => { pendingTypeChange.apply(); setPendingTypeChange(null); }}
        />
      )}

      {duplicating && selectedProject && selectedEnv && (
        <DuplicateEnvModal
          source={selectedEnv}
          usedNames={selectedProject.environments.map((e) => e.name)}
          onCancel={() => setDuplicating(false)}
          onConfirm={handleDuplicateEnvironment}
        />
      )}

      {deleteVarTarget && itemsById.get(deleteVarTarget.itemId) && (
        <ConfirmDeleteVarModal
          varKey={deleteVarTarget.key}
          item={itemsById.get(deleteVarTarget.itemId)!}
          onCancel={() => setDeleteVarTarget(null)}
          onConfirm={() => handleDeleteVar(deleteVarTarget)}
        />
      )}

      {detailVar && itemsById.get(detailVar.itemId) && (
        <VarDetailModal
          varKey={detailVar.key}
          item={itemsById.get(detailVar.itemId)!}
          onClose={() => setDetailVar(null)}
          onToggleGlobal={(global) => handleToggleVarGlobal(detailVar, global)}
        />
      )}

      {injectTargets && (
        <InjectTargetModal
          paths={injectTargets.paths}
          onCancel={() => settleInjectTargets('cancel')}
          onConfirm={(targets) => settleInjectTargets(targets)}
        />
      )}

      {manifestPrompt !== null && (
        <ManifestExistsModal
          onLink={() => setManifestPrompt(null)}
          onOverwrite={async () => {
            const id = manifestPrompt;
            setManifestPrompt(null);
            await writeManifest(id, true);
          }}
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
