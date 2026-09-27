import { useState } from 'react';
import { Icon } from './ui/Icon';
import { TagInput } from './ui/TagInput';
import { useVaultStore } from '../store';
import { useTranslation } from '../i18n';
import type { ItemType, VaultItem } from '../types';
import {
  F, Label, ItemTypePicker, ItemTypeFields, emptyItemFields, validateItemFields,
  type ItemFieldsState,
} from './itemFields/ItemTypeFields';

interface FormState extends ItemFieldsState {
  categories: string[];
  notes:      string;
  isGlobal:   boolean;
}

const emptyForm = (): FormState => ({
  ...emptyItemFields(),
  categories: [], notes: '', isGlobal: false,
});

function fromItem(item: VaultItem): FormState {
  const base = emptyForm();
  Object.assign(base, item);
  if ('title'       in item) base.title       = item.title;
  if ('description' in item) base.description = item.description ?? '';
  base.isGlobal = item.isGlobal ?? false;
  return base;
}

export function EditItem() {
  const cats          = useVaultStore((s) => s.cats);
  const editTarget     = useVaultStore((s) => s.editTarget);
  const goBack         = useVaultStore((s) => s.goBack);
  const saveItem       = useVaultStore((s) => s.saveItem);
  const deleteItem     = useVaultStore((s) => s.deleteItem);
  const showToast      = useVaultStore((s) => s.showToast);
  const toggleGlobal   = useVaultStore((s) => s.toggleGlobal);
  const getItemOwners  = useVaultStore((s) => s.getItemOwners);
  const { t }          = useTranslation();

  const isNew   = !editTarget;
  const defType = (editTarget?.type ?? 'secret') as ItemType;

  const [type,       setType]       = useState<ItemType>(defType);
  const [form,       setForm]       = useState<FormState>(editTarget ? fromItem(editTarget) : emptyForm());
  const [showVal,    setShowVal]    = useState(false);
  const [saving,     setSaving]     = useState(false);
  const [errors,     setErrors]     = useState<Record<string, string>>({});
  const [confirmDel, setConfirmDel] = useState(false);
  const [forkWarning, setForkWarning] = useState<{ owners: number } | null>(null);

  const set = (k: keyof FormState, v: string | string[] | boolean) =>
    setForm((f) => ({ ...f, [k]: v }));

  const clearError = (k: string) => setErrors((r) => ({ ...r, [k]: '' }));

  const validate = () => validateItemFields(type, form);

  const handleSave = async () => {
    const e = validate();
    if (Object.keys(e).length) { setErrors(e); return; }
    setSaving(true);
    try {
      const payload = { ...form, type } as Omit<VaultItem, 'id' | 'created'>;
      await saveItem(payload);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      showToast(msg || t('edit.saveFailed'), 'error');
    } finally {
      setSaving(false);
    }
  };

  // Un-globaling an item with >1 owners forks it into independent copies —
  // route that specific transition through toggleGlobal (not the normal
  // save path) and warn the user first.
  const handleGlobalToggle = async (checked: boolean) => {
    if (checked || isNew || !editTarget) { set('isGlobal', checked); return; }
    try {
      const owners = await getItemOwners(editTarget.id);
      if (owners.length > 1) {
        setForkWarning({ owners: owners.length });
        return;
      }
      set('isGlobal', checked);
    } catch {
      set('isGlobal', checked);
    }
  };

  const confirmFork = async () => {
    if (!editTarget) return;
    setForkWarning(null);
    setSaving(true);
    try {
      await toggleGlobal(editTarget.id, false);
      showToast(t('edit.splitDone'));
      goBack();
    } catch (err) {
      showToast(err instanceof Error ? err.message : String(err), 'error');
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="flex-1 flex flex-col overflow-hidden relative animate-fade-in">
      {/* Header */}
      <div className="px-3.5 py-[9px] border-b border-bd flex items-center gap-[10px] shrink-0">
        <button
          onClick={goBack}
          className="flex items-center gap-1 text-[12px] font-medium font-ui text-tx3 bg-transparent border-none cursor-pointer hover:text-tx transition-colors"
        >
          <Icon name="back" size={13} />{t('common.back')}
        </button>
        <div className="flex-1 text-[13px] font-semibold text-center text-tx">
          {isNew ? t('edit.newItem') : t('edit.editItem')}
        </div>
        {!isNew && (
          <button
            onClick={() => setConfirmDel(true)}
            aria-label={t('common.deleteLower')}
            title={t('common.deleteLower')}
            className="bg-transparent border-none cursor-pointer text-tx3 flex p-[2px] hover:text-danger transition-colors"
          >
            <Icon name="trash" size={13} />
          </button>
        )}
      </div>

      {/* Form body */}
      <div className="flex-1 overflow-y-auto p-4 bg-surface">
        {isNew && (
          <ItemTypePicker type={type} onSelect={(nt) => { setType(nt); setErrors({}); }} />
        )}

        <ItemTypeFields
          type={type}
          form={form}
          errors={errors}
          showVal={showVal}
          setShowVal={setShowVal}
          set={(k, v) => set(k, v)}
          clearError={clearError}
        />

        {/* Shared: Categories + Global + Notes */}
        <F><Label label={t('edit.categories')} /><TagInput selected={form.categories} categories={cats} onChange={(v) => set('categories', v)} /></F>
        <F>
          <label className="flex items-center gap-2 cursor-pointer select-none">
            <input
              type="checkbox"
              checked={form.isGlobal}
              onChange={(e) => handleGlobalToggle(e.target.checked)}
              className="accent-[var(--color-accent)]"
            />
            <span className="text-[11px] font-semibold tracking-[0.07em] text-tx">{t('edit.addToGlobal')}</span>
          </label>
          <div className="mt-1 text-[11px] text-tx2 leading-[1.5]">
            {t('edit.globalHint')}
          </div>
        </F>
        <F cls="mb-0">
          <Label label={t('edit.notes')} />
          <textarea value={form.notes} onChange={(e) => set('notes', e.target.value)}
            placeholder={t('edit.notesPlaceholder')} rows={2}
            className="w-full px-[10px] py-2 text-[12px] font-ui bg-raised border border-bd2 rounded-[3px] text-tx placeholder:text-tx3 transition-[border-color] duration-150 outline-none focus:border-accent-d resize-none leading-[1.5]" />
        </F>
        {!isNew && (
          <div className="mt-2 text-[11px] text-tx3 font-mono">// {t('edit.created', { date: editTarget!.created })}</div>
        )}
      </div>

      {/* Footer */}
      <div className="px-3.5 py-[10px] border-t border-bd flex gap-[7px] shrink-0 bg-bg">
        <button onClick={goBack}
          className="flex-1 py-[9px] bg-transparent border border-bd2 rounded-[3px] text-tx2 text-[12px] font-semibold tracking-[0.05em] cursor-pointer font-ui hover:text-tx transition-colors">
          {t('common.cancel')}
        </button>
        <button onClick={handleSave} disabled={saving}
          className={[
            'flex-[2] py-[9px] border-none rounded-[3px]',
            'text-[12px] font-bold tracking-[0.06em] cursor-pointer font-ui',
            'flex items-center justify-center gap-1.5 transition-[background] duration-150',
            saving ? 'bg-accent-d text-[#020504]' : 'bg-accent text-[#020504] hover:opacity-90',
          ].join(' ')}>
          {saving ? (
            <><div className="w-3 h-3 rounded-full border-2 border-transparent border-t-[#020504] animate-spin-fast" />{t('common.saving')}</>
          ) : (
            isNew ? t('edit.addItem') : t('edit.saveChanges')
          )}
        </button>
      </div>

      {/* Delete confirm overlay */}
      {confirmDel && (
        <div className="absolute inset-0 bg-[rgba(10,11,14,.85)] flex items-center justify-center p-6 z-[100] backdrop-blur-[4px]">
          <div className="bg-surface border border-danger rounded-[4px] p-[22px] w-full">
            <div className="text-[14px] font-bold mb-2 text-tx">{t('edit.deleteTitle')}</div>
            <div className="text-[12px] text-tx3 mb-[18px] leading-[1.5]">
              <span className="font-mono text-tx">
                {editTarget && ('name' in editTarget ? editTarget.name : 'title' in editTarget ? editTarget.title : '')}
              </span>{' '}
              {t('edit.deleteBody')}
            </div>
            <div className="flex gap-2">
              <button onClick={() => setConfirmDel(false)}
                className="flex-1 py-2 bg-transparent border border-bd2 rounded-[3px] text-tx2 text-[12px] cursor-pointer font-ui">
                {t('common.cancel')}
              </button>
              <button onClick={async () => { await deleteItem(editTarget!.id); }}
                className="flex-1 py-2 bg-danger border-none rounded-[3px] text-white text-[12px] font-bold cursor-pointer font-ui">
                {t('common.delete')}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Un-global fork warning overlay */}
      {forkWarning && (
        <div className="absolute inset-0 bg-[rgba(10,11,14,.85)] flex items-center justify-center p-6 z-[100] backdrop-blur-[4px]">
          <div className="bg-surface border border-danger rounded-[4px] p-[22px] w-full">
            <div className="text-[14px] font-bold mb-2 text-tx">{t('edit.splitTitle', { n: forkWarning.owners })}</div>
            <div className="text-[12px] text-tx3 mb-[18px] leading-[1.5]">
              {t('edit.splitBody', { n: forkWarning.owners })}
            </div>
            <div className="flex gap-2">
              <button onClick={() => setForkWarning(null)}
                className="flex-1 py-2 bg-transparent border border-bd2 rounded-[3px] text-tx2 text-[12px] cursor-pointer font-ui">
                {t('common.cancel')}
              </button>
              <button onClick={confirmFork}
                className="flex-1 py-2 bg-danger border-none rounded-[3px] text-white text-[12px] font-bold cursor-pointer font-ui">
                {t('edit.split')}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
