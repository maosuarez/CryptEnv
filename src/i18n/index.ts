import { create } from 'zustand';
import en from './locales/en.json';
import esJson from './locales/es.json';
import ptJson from './locales/pt.json';

/** English is the reference dictionary: every other locale must have the same
 *  shape, which the `Dict` annotations below enforce at compile time. */
export type Dict = typeof en;
const es: Dict = esJson;
const pt: Dict = ptJson;

export const LANGUAGES = ['en', 'es', 'pt'] as const;
export type Language = (typeof LANGUAGES)[number];

/** Native names, shown untranslated in the language selector. */
export const LANGUAGE_NAMES: Record<Language, string> = {
  en: 'English',
  es: 'Español',
  pt: 'Português',
};

const DICTS: Record<Language, Dict> = { en, es, pt };

export const LANGUAGE_STORAGE_KEY = 'cryptenv_language';

/** Dotted paths to every string leaf of `Dict`, e.g. `'settings.title'`. */
type Leaves<T> = {
  [K in keyof T & string]: T[K] extends string ? K : `${K}.${Leaves<T[K]>}`;
}[keyof T & string];
export type TKey = Leaves<Dict>;

export type TParams = Record<string, string | number>;

function lookup(dict: Dict, key: string): string | undefined {
  let node: unknown = dict;
  for (const part of key.split('.')) {
    if (typeof node !== 'object' || node === null) return undefined;
    node = (node as Record<string, unknown>)[part];
  }
  return typeof node === 'string' ? node : undefined;
}

/** Replaces `{name}` placeholders; unknown placeholders are left intact. */
export function interpolate(template: string, params?: TParams): string {
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (m, name: string) =>
    name in params ? String(params[name]) : m,
  );
}

/** Resolves `key` in `lang`, falling back to English and then to the key itself. */
export function translate(lang: Language, key: TKey, params?: TParams): string {
  const template = lookup(DICTS[lang], key) ?? lookup(en, key) ?? key;
  return interpolate(template, params);
}

function isLanguage(v: unknown): v is Language {
  return typeof v === 'string' && (LANGUAGES as readonly string[]).includes(v);
}

/** Saved preference, else the system locale when it is en/es/pt, else English. */
export function detectInitialLanguage(): Language {
  try {
    const saved = localStorage.getItem(LANGUAGE_STORAGE_KEY);
    if (isLanguage(saved)) return saved;
  } catch {}
  const system = (typeof navigator !== 'undefined' ? navigator.language : '').slice(0, 2).toLowerCase();
  return isLanguage(system) ? system : 'en';
}

interface LanguageStore {
  lang:    Language;
  setLang: (lang: Language) => void;
}

export const useLanguageStore = create<LanguageStore>((set) => ({
  lang: detectInitialLanguage(),
  setLang: (lang) => {
    try {
      localStorage.setItem(LANGUAGE_STORAGE_KEY, lang);
    } catch {}
    document.documentElement.lang = lang;
    set({ lang });
  },
}));

document.documentElement.lang = useLanguageStore.getState().lang;

/** Non-hook translator for code outside React render (stores, callbacks). */
export function t(key: TKey, params?: TParams): string {
  return translate(useLanguageStore.getState().lang, key, params);
}

/** Re-renders the caller when the language changes. */
export function useTranslation() {
  const lang    = useLanguageStore((s) => s.lang);
  const setLang = useLanguageStore((s) => s.setLang);
  const tr = (key: TKey, params?: TParams) => translate(lang, key, params);
  return { t: tr, lang, setLang };
}
