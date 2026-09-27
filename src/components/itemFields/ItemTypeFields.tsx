import { Icon } from '../ui/Icon';
import type { ItemType, Shell } from '../../types';
import { t as tr, useTranslation } from '../../i18n';

export const TYPE_META: Record<ItemType, { label: string; abbr: string; dot: string }> = {
  secret:     { label: 'Secret',     abbr: 'KEY',  dot: 'oklch(0.70 0.17 162)' },
  credential: { label: 'Credential', abbr: 'CRED', dot: 'oklch(0.70 0.15 220)' },
  link:       { label: 'Link',       abbr: 'LINK', dot: 'oklch(0.68 0.15 270)' },
  command:    { label: 'Command',    abbr: 'CMD',  dot: 'oklch(0.72 0.16 68)'  },
  note:       { label: 'Note',       abbr: 'NOTE', dot: 'oklch(0.72 0.15 350)' },
};

export const SHELLS: Shell[] = ['bash', 'zsh', 'fish', 'PowerShell', 'cmd'];

export function Label({ label, err }: { label: string; err?: string }) {
  return (
    <div className={`text-[11px] font-semibold tracking-[0.07em] mb-[5px] ${err ? 'text-danger' : 'text-tx'}`}>
      {label}
      {err && <span className="ml-2 normal-case font-mono">// {err}</span>}
    </div>
  );
}

export function F({ children, cls = '' }: { children: React.ReactNode; cls?: string }) {
  return <div className={`mb-[13px] ${cls}`}>{children}</div>;
}

export interface ItemFieldsState {
  name:        string;
  value:       string;
  url:         string;
  username:    string;
  password:    string;
  title:       string;
  description: string;
  command:     string;
  shell:       Shell;
  content:     string;
}

export const emptyItemFields = (): ItemFieldsState => ({
  name: '', value: '', url: '', username: '', password: '',
  title: '', description: '', command: '', shell: 'bash', content: '',
});

export function validateItemFields(type: ItemType, form: ItemFieldsState): Record<string, string> {
  const e: Record<string, string> = {};
  if (type === 'secret'     && !form.name.trim())     e.name     = tr('itemFields.required');
  if (type === 'secret'     && !form.value.trim())    e.value    = tr('itemFields.required');
  if (type === 'credential' && !form.name.trim())     e.name     = tr('itemFields.required');
  if (type === 'credential' && !form.username.trim()) e.username = tr('itemFields.required');
  if (type === 'link'       && !form.title.trim())    e.title    = tr('itemFields.required');
  if (type === 'link'       && !form.url.trim())      e.url      = tr('itemFields.required');
  if (type === 'command'    && !form.name.trim())     e.name     = tr('itemFields.required');
  if (type === 'command'    && !form.command.trim())  e.command  = tr('itemFields.required');
  if (type === 'note'       && !form.title.trim())    e.title    = tr('itemFields.required');
  if (type === 'note'       && !form.content.trim())  e.content  = tr('itemFields.required');
  return e;
}

const iBase = (err?: string) =>
  [
    'w-full px-[10px] py-2 text-[12px] font-ui bg-raised border rounded-[3px]',
    'text-tx placeholder:text-tx3 transition-[border-color] duration-150 outline-none',
    'focus:border-accent-d',
    err ? 'border-danger' : 'border-bd2',
  ].join(' ');

export function ItemTypePicker({
  type,
  onSelect,
}: {
  type:     ItemType;
  onSelect: (t: ItemType) => void;
}) {
  const { t } = useTranslation();
  return (
    <F>
      <Label label={t('itemFields.itemType')} />
      <div className="grid grid-cols-5 gap-[5px]">
        {(Object.entries(TYPE_META) as [ItemType, typeof TYPE_META[ItemType]][]).map(([k, m]) => (
          <button
            key={k}
            onClick={() => onSelect(k)}
            className={[
              'py-[7px] px-1 rounded-[3px] border cursor-pointer text-[11px] font-medium font-ui',
              'flex flex-col items-center gap-1 transition-all duration-100',
              type === k
                ? 'bg-accent-b border-accent-d text-accent'
                : 'bg-raised border-bd2 text-tx2 hover:text-tx',
            ].join(' ')}
          >
            <span className="w-[6px] h-[6px] rounded-full" style={{ background: m.dot }} />
            <span className="text-[10px] tracking-[0.04em]">{t(`itemFields.abbr.${k}`)}</span>
          </button>
        ))}
      </div>
    </F>
  );
}

export function ItemTypeFields({
  type,
  form,
  errors,
  showVal,
  setShowVal,
  set,
  clearError,
}: {
  type:       ItemType;
  form:       ItemFieldsState;
  errors:     Record<string, string>;
  showVal:    boolean;
  setShowVal: (fn: (v: boolean) => boolean) => void;
  set:        (k: keyof ItemFieldsState, v: string) => void;
  clearError: (k: string) => void;
}) {
  const { t } = useTranslation();
  if (type === 'secret') {
    return (
      <>
        <F>
          <Label label={t('itemFields.name')} err={errors.name} />
          <input value={form.name} onChange={(e) => { set('name', e.target.value); clearError('name'); }}
            placeholder={t('itemFields.phSecretName')} className={`${iBase(errors.name)} font-mono tracking-[0.04em]`} />
        </F>
        <F>
          <Label label={t('itemFields.value')} err={errors.value} />
          <div className={`flex items-center border rounded-[3px] bg-raised ${errors.value ? 'border-danger' : 'border-bd2'}`}>
            <input type={showVal ? 'text' : 'password'} value={form.value}
              onChange={(e) => { set('value', e.target.value); clearError('value'); }}
              placeholder="sk-…" className="flex-1 px-[10px] py-2 text-[12px] font-mono text-tx bg-transparent border-none outline-none" />
            <button onClick={() => setShowVal((v) => !v)} aria-label={showVal ? t('common.hide') : t('common.show')} className="bg-transparent border-none cursor-pointer px-[9px] text-tx3 flex hover:text-tx">
              <Icon name={showVal ? 'eyeOff' : 'eye'} size={12} />
            </button>
          </div>
        </F>
      </>
    );
  }

  if (type === 'credential') {
    return (
      <>
        <F><Label label={t('itemFields.siteName')} err={errors.name} />
          <input value={form.name} onChange={(e) => { set('name', e.target.value); clearError('name'); }}
            placeholder={t('itemFields.phSite')} className={iBase(errors.name)} /></F>
        <F><Label label={t('itemFields.url')} />
          <input value={form.url} onChange={(e) => set('url', e.target.value)} placeholder="https://"
            className={`${iBase()} font-mono text-[11px]`} /></F>
        <F><Label label={t('itemFields.username')} err={errors.username} />
          <input value={form.username} onChange={(e) => { set('username', e.target.value); clearError('username'); }}
            placeholder="user@email.com" className={`${iBase(errors.username)} font-mono text-[11px]`} /></F>
        <F><Label label={t('itemFields.password')} />
          <div className="flex items-center border border-bd2 rounded-[3px] bg-raised">
            <input type={showVal ? 'text' : 'password'} value={form.password} onChange={(e) => set('password', e.target.value)}
              placeholder="••••••••" className="flex-1 px-[10px] py-2 text-[12px] font-mono text-tx bg-transparent border-none outline-none" />
            <button onClick={() => setShowVal((v) => !v)} aria-label={showVal ? t('common.hide') : t('common.show')} className="bg-transparent border-none cursor-pointer px-[9px] text-tx3 flex hover:text-tx">
              <Icon name={showVal ? 'eyeOff' : 'eye'} size={12} />
            </button>
          </div></F>
      </>
    );
  }

  if (type === 'link') {
    return (
      <>
        <F><Label label={t('itemFields.title')} err={errors.title} />
          <input value={form.title} onChange={(e) => { set('title', e.target.value); clearError('title'); }}
            placeholder={t('itemFields.phLinkTitle')} className={iBase(errors.title)} /></F>
        <F><Label label={t('itemFields.url')} err={errors.url} />
          <input value={form.url} onChange={(e) => { set('url', e.target.value); clearError('url'); }}
            placeholder="https://" className={`${iBase(errors.url)} font-mono text-[11px]`} /></F>
        <F><Label label={t('itemFields.description')} />
          <input value={form.description} onChange={(e) => set('description', e.target.value)}
            placeholder={t('itemFields.phShortDesc')} className={iBase()} /></F>
      </>
    );
  }

  if (type === 'command') {
    return (
      <>
        <F><Label label={t('itemFields.name')} err={errors.name} />
          <input value={form.name} onChange={(e) => { set('name', e.target.value); clearError('name'); }}
            placeholder={t('itemFields.phCommandName')} className={iBase(errors.name)} /></F>
        <F>
          <Label label={t('itemFields.command')} err={errors.command} />
          <div className={`border rounded-[3px] bg-raised px-[10px] py-2 ${errors.command ? 'border-danger' : 'border-bd2'}`}>
            <textarea value={form.command}
              onChange={(e) => { set('command', e.target.value); clearError('command'); }}
              placeholder={'ssh ubuntu@{{HOST}}\npg_dump … > {{OUT}}.sql'} rows={3}
              className="w-full resize-none text-[11px] font-mono text-tx placeholder:text-tx3 leading-[1.6] bg-transparent border-none outline-none" />
          </div>
          <div className="mt-1 text-[11px] text-tx2">
            {t('itemFields.placeholderHintPre')}<span className="font-mono text-warn">{'{{PLACEHOLDER}}'}</span>{t('itemFields.placeholderHintPost')}
          </div>
        </F>
        <F>
          <Label label={t('itemFields.shell')} />
          <div className="flex gap-[5px]">
            {SHELLS.map((s) => (
              <button key={s} onClick={() => set('shell', s)}
                className={[
                  'flex-1 py-[5px] px-1 rounded-[3px] border cursor-pointer text-[10px] font-mono transition-all duration-100',
                  form.shell === s ? 'bg-accent-b border-accent-d text-accent' : 'bg-raised border-bd2 text-tx2 hover:text-tx',
                ].join(' ')}>
                {s}
              </button>
            ))}
          </div>
        </F>
        <F><Label label={t('itemFields.description')} />
          <input value={form.description} onChange={(e) => set('description', e.target.value)}
            placeholder={t('itemFields.phCommandDesc')} className={iBase()} /></F>
      </>
    );
  }

  if (type === 'note') {
    return (
      <>
        <F><Label label={t('itemFields.title')} err={errors.title} />
          <input value={form.title} onChange={(e) => { set('title', e.target.value); clearError('title'); }}
            placeholder={t('itemFields.phNoteTitle')} className={iBase(errors.title)} /></F>
        <F>
          <Label label={t('itemFields.content')} err={errors.content} />
          <div className={`border rounded-[3px] bg-raised px-[10px] py-2 ${errors.content ? 'border-danger' : 'border-bd2'}`}>
            <textarea value={form.content}
              onChange={(e) => { set('content', e.target.value); clearError('content'); }}
              placeholder={t('itemFields.phNoteContent')} rows={6}
              className="w-full resize-none text-[12px] font-ui text-tx placeholder:text-tx3 leading-[1.6] bg-transparent border-none outline-none" />
          </div>
        </F>
      </>
    );
  }

  return null;
}
