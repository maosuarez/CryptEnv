import { useState, useMemo, useRef, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Icon } from './ui/Icon';
import { SecretRow } from './rows/SecretRow';
import { CredentialRow } from './rows/CredentialRow';
import { LinkRow } from './rows/LinkRow';
import { CommandRow } from './rows/CommandRow';
import { NoteRow } from './rows/NoteRow';
import { ShareModal } from './ShareModal';
import { useVaultStore } from '../store';
import { useTranslation, type TKey } from '../i18n';
import type { ItemType, VaultItem, Category } from '../types';

type TypeFilter = 'all' | ItemType;

const TYPE_PILLS: { id: TypeFilter; label: TKey; dot?: string }[] = [
  { id: 'all',        label: 'secrets.pill.all' },
  { id: 'secret',     label: 'secrets.pill.secret', dot: 'oklch(0.70 0.17 162)' },
  { id: 'credential', label: 'secrets.pill.credential', dot: 'oklch(0.70 0.15 220)' },
  { id: 'link',       label: 'secrets.pill.link', dot: 'oklch(0.68 0.15 270)' },
  { id: 'command',    label: 'secrets.pill.command', dot: 'oklch(0.72 0.16 68)'  },
  { id: 'note',       label: 'secrets.pill.note', dot: 'oklch(0.72 0.15 350)' },
];

export function GlobalSecrets() {
  const allItems = useVaultStore((s) => s.items);
  const cats  = useVaultStore((s) => s.cats);
  const go    = useVaultStore((s) => s.go);
  const { t } = useTranslation();

  // Only secrets explicitly marked global live here — everything else is
  // created and managed inside a project's environments.
  const items = useMemo(() => allItems.filter((i) => i.isGlobal), [allItems]);

  const [query,      setQuery]      = useState('');
  const [typeF,      setTypeF]      = useState<TypeFilter>('all');
  const [catF,       setCatF]       = useState<Set<string>>(new Set());
  const [catOpen,    setCatOpen]    = useState(false);
  const [loading,    setLoading]    = useState(true);
  const [shareMode,  setShareMode]  = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<number>>(new Set());
  const [showShareModal, setShowShareModal] = useState(false);
  const [reloading,  setReloading]  = useState(false);
  const ref       = useRef<HTMLInputElement>(null);
  const catBtnRef = useRef<HTMLButtonElement>(null);
  const catMenuRef = useRef<HTMLDivElement>(null);

  const toggleItem = useCallback((id: number) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const exitShareMode = useCallback(() => {
    setShareMode(false);
    setSelectedIds(new Set());
  }, []);

  const handleReload = useCallback(async () => {
    setReloading(true);
    try {
      const result = await invoke<{ items: VaultItem[]; categories: Category[] }>('vault_list');
      useVaultStore.setState({ items: result.items, cats: result.categories });
    } catch (e) {
      console.error('Reload failed:', e);
    } finally {
      setReloading(false);
    }
  }, []);

  const handleShareItem = useCallback((id: number) => {
    setSelectedIds(new Set([id]));
    setShowShareModal(true);
  }, []);

  const handleSelectItem = useCallback((id: number) => {
    setShareMode(true);
    setSelectedIds((prev) => {
      const next = new Set(prev);
      next.add(id);
      return next;
    });
  }, []);

  useEffect(() => {
    setTimeout(() => setLoading(false), 400);
    ref.current?.focus();
  }, []);

  useEffect(() => {
    if (!catOpen) return;
    const handler = (e: MouseEvent) => {
      if (
        catMenuRef.current && !catMenuRef.current.contains(e.target as Node) &&
        catBtnRef.current && !catBtnRef.current.contains(e.target as Node)
      ) {
        setCatOpen(false);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [catOpen]);

  const filtered = useMemo(() => {
    return items.filter((it) => {
      if (typeF !== 'all' && it.type !== typeF) return false;
      if (catF.size > 0 && !(it.categories ?? []).some((c) => catF.has(c))) return false;
      const q = query.toLowerCase();
      if (!q) return true;
      const name = (('name' in it ? it.name : 'title' in it ? it.title : '') as string).toLowerCase();
      const val  = (('value' in it ? it.value : 'url' in it ? it.url : 'command' in it ? it.command : 'content' in it ? it.content : '') as string).toLowerCase();
      const desc = (it.notes ?? ('description' in it ? (it as any).description : '')).toLowerCase();
      const user = (('username' in it ? it.username : '') as string).toLowerCase();
      return name.includes(q) || val.includes(q) || desc.includes(q) || user.includes(q) || (it.categories ?? []).join(' ').toLowerCase().includes(q);
    });
  }, [items, typeF, catF, query]);

  const typeCounts = useMemo(() => {
    const c = { all: items.length, secret: 0, credential: 0, link: 0, command: 0, note: 0 };
    items.forEach((it) => { (c as any)[it.type] = ((c as any)[it.type] ?? 0) + 1; });
    return c;
  }, [items]);

  return (
    <div className="flex-1 flex flex-col overflow-hidden animate-fade-in">
      {/* Header */}
      <div className="px-3.5 py-[9px] border-b border-bd flex items-center gap-[10px] shrink-0">
        <button
          onClick={() => go('projects')}
          className="flex items-center gap-1 text-[12px] font-medium font-ui text-tx3 bg-transparent border-none cursor-pointer hover:text-tx transition-colors"
        >
          <Icon name="back" size={13} />{t('secrets.backToProjects')}
        </button>
        <div className="flex-1 text-[13px] font-semibold text-center text-tx">{t('secrets.title')}</div>
        <span className="text-[11px] text-tx3 font-mono">{items.length}</span>
      </div>

      {/* Search bar */}
      <div className="flex items-center gap-3 pl-5 pr-4 h-12 border-b border-bd bg-bg shrink-0">
        <Icon name="search" size={16} />
        <input
          ref={ref}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t('secrets.searchPlaceholder')}
          className="flex-1 text-[13px] text-tx font-ui bg-transparent outline-none placeholder:text-tx3"
        />
        {query && (
          <button
            onClick={() => setQuery('')}
            aria-label={t('secrets.clearSearch')}
            className="flex items-center justify-center w-6 h-6 rounded-[3px] text-tx3 hover:text-tx hover:bg-raised transition"
          >
            <Icon name="close" size={14} />
          </button>
        )}
        <div className="relative">
          <button
            ref={catBtnRef}
            onClick={() => setCatOpen((v) => !v)}
            title={t('secrets.filterByCategory')}
            className={[
              'flex items-center justify-center w-6 h-6 rounded-[3px] transition',
              catF.size > 0
                ? 'text-accent bg-accent-b hover:opacity-80'
                : 'text-tx3 hover:text-tx hover:bg-raised',
            ].join(' ')}
          >
            <Icon name="funnel" size={14} />
          </button>
          {catOpen && (
            <div
              ref={catMenuRef}
              className="absolute right-0 top-8 z-50 min-w-[180px] bg-bg border border-bd rounded-[3px] shadow-lg py-1 flex flex-col"
            >
              <div className="px-3 py-1.5 text-[0.6rem] font-mono text-tx3 tracking-[0.1em] border-b border-bd">
                {t('secrets.filterByCategoryCaps')}
              </div>
              {cats.length === 0 ? (
                <div className="px-3 py-2 text-xs text-tx3 italic">{t('secrets.noCategories')}</div>
              ) : (
                cats.map((cat) => {
                  const active = catF.has(cat.name);
                  return (
                    <button
                      key={cat.id}
                      onClick={() => {
                        setCatF((prev) => {
                          const next = new Set(prev);
                          if (next.has(cat.name)) next.delete(cat.name);
                          else next.add(cat.name);
                          return next;
                        });
                      }}
                      className={[
                        'flex items-center gap-2 px-3 py-1.5 text-left w-full',
                        'text-[12px] font-ui transition-colors duration-100',
                        active ? 'bg-raised text-tx' : 'text-tx2 hover:bg-raised hover:text-tx',
                      ].join(' ')}
                    >
                      <span
                        className="w-2 h-2 rounded-full shrink-0"
                        style={{ background: cat.color }}
                      />
                      <span className="flex-1 truncate">{cat.name}</span>
                      {active && (
                        <svg width="10" height="10" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                          <path d="M2.5 8.5l4 4 7-8" />
                        </svg>
                      )}
                    </button>
                  );
                })
              )}
              {catF.size > 0 && (
                <>
                  <div className="border-t border-bd mt-1" />
                  <button
                    onClick={() => setCatF(new Set())}
                    className="flex items-center gap-2 px-3 py-1.5 text-[11px] font-ui text-tx3 hover:text-tx hover:bg-raised transition-colors w-full text-left"
                  >
                    <Icon name="close" size={11} />
                    {t('secrets.clearFilter')}
                  </button>
                </>
              )}
            </div>
          )}
        </div>
        <button
          onClick={handleReload}
          disabled={reloading}
          title={t('secrets.reload')}
          className="flex items-center justify-center w-6 h-6 rounded-[3px] text-tx3 hover:text-tx hover:bg-raised transition disabled:opacity-50"
        >
          <span className={reloading ? 'animate-spin inline-flex' : 'inline-flex'}>
            <Icon name="refresh" size={14} />
          </span>
        </button>
        <span className="text-[11px] text-tx2 font-mono min-w-[24px] text-right">
          {filtered.length}
        </span>
      </div>

      {/* Type filter pills + action buttons */}
      <div className="px-3 h-10 border-b border-bd bg-bg flex items-center gap-1.5 shrink-0">
        {!shareMode ? (
          <>
            {TYPE_PILLS.map(({ id, label, dot }) => {
              const active = typeF === id;
              const count  = (typeCounts as any)[id] ?? 0;
              return (
                <button
                  key={id}
                  onClick={() => setTypeF(id)}
                  className={[
                    'flex items-center gap-1 rounded-[3px] px-2.5 h-[28px]',
                    'border text-[11px] font-medium tracking-[0.05em] font-ui cursor-pointer',
                    'transition-all duration-150 shrink-0 whitespace-nowrap',
                    active
                      ? 'bg-accent-b border-accent-d text-accent'
                      : 'bg-raised border-bd2 text-tx2 hover:border-accent-d hover:text-tx',
                  ].join(' ')}
                >
                  {dot && !active && (
                    <span className="w-1.5 h-1.5 rounded-full shrink-0" style={{ background: dot }} />
                  )}
                  {t(label)}
                  <span className="text-[0.6rem] opacity-50">{count}</span>
                </button>
              );
            })}

            <div className="flex-1" />
          </>
        ) : (
          /* Share mode bar */
          <>
            <span className="text-[10px] font-mono text-accent tracking-[0.08em] shrink-0">
              {t('secrets.selectToShare')}
            </span>

            <div className="flex-1" />

            {selectedIds.size > 0 && (
              <span className="text-[10px] font-mono text-tx3 shrink-0">
                {t('secrets.selected', { n: selectedIds.size })}
              </span>
            )}

            <button
              onClick={exitShareMode}
              className={[
                'flex items-center gap-1 border rounded-[3px]',
                'px-2.5 h-[28px] text-[10px] font-medium tracking-wider font-ui',
                'border-bd2 text-tx2 bg-transparent cursor-pointer shrink-0',
                'hover:border-tx3 hover:text-tx transition-all duration-150',
              ].join(' ')}
            >
              {t('common.cancel')}
            </button>

            <button
              onClick={() => setShowShareModal(true)}
              className={[
                'flex items-center gap-1 border-none rounded-[3px]',
                'px-2.5 h-[28px] text-[10px] font-bold tracking-wider font-ui',
                'cursor-pointer shrink-0 transition-opacity',
                selectedIds.size > 0
                  ? 'bg-accent text-[#020504] hover:opacity-90'
                  : 'bg-raised text-tx3 opacity-60 cursor-not-allowed',
              ].join(' ')}
              disabled={selectedIds.size === 0}
            >
              <Icon name="export" size={11} color={selectedIds.size > 0 ? '#020504' : 'currentColor'} />
              {selectedIds.size > 0 ? t('secrets.shareCount', { n: selectedIds.size }) : t('secrets.share')}
            </button>
          </>
        )}
      </div>

      {/* Item list */}
      <div className="flex-1 overflow-y-auto bg-surface">
        {loading ? (
          <div className="py-16 text-center text-tx3 text-sm font-mono">
            <div className="w-5 h-5 rounded-full border-2 border-bd2 border-t-accent animate-spin-fast mx-auto mb-3" />
            {t('secrets.decrypting')}
          </div>
        ) : filtered.length === 0 ? (
          <div className="py-16 text-center text-tx3 text-sm font-mono leading-[1.8]">
            // {t('secrets.noMatch')}<br />
            <span className="text-[11px]">{t('secrets.noMatchHint')}</span>
          </div>
        ) : (
          filtered.map((it) => {
            const selProps = {
              selected: shareMode ? selectedIds.has(it.id) : undefined,
              onToggle: shareMode ? toggleItem : undefined,
              onShare: handleShareItem,
              onSelect: handleSelectItem,
            };
            const props = { key: it.id, cats, ...selProps };
            if (it.type === 'secret')     return <SecretRow     {...props} item={it} />;
            if (it.type === 'credential') return <CredentialRow {...props} item={it} />;
            if (it.type === 'link')       return <LinkRow       {...props} item={it} />;
            if (it.type === 'command')    return <CommandRow    {...props} item={it} />;
            if (it.type === 'note')       return <NoteRow       {...props} item={it} />;
            return null;
          })
        )}
      </div>

      {/* Status bar */}
      <div className="flex items-center px-5 h-10 border-t border-bd bg-bg shrink-0">
        <div className="text-[12px] text-tx2 font-mono">{t(items.length === 1 ? 'secrets.count_one' : 'secrets.count_other', { n: items.length })} · AES-256-GCM</div>
      </div>

      {/* Share modal overlay */}
      {showShareModal && (
        <ShareModal
          selectedIds={Array.from(selectedIds)}
          onClose={() => setShowShareModal(false)}
          onImportDone={() => { setShowShareModal(false); exitShareMode(); }}
          onSendDone={() => exitShareMode()}
        />
      )}
    </div>
  );
}
