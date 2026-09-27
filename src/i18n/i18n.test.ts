import { beforeEach, describe, expect, it } from 'vitest';
import en from './locales/en.json';
import es from './locales/es.json';
import pt from './locales/pt.json';
import {
  LANGUAGE_STORAGE_KEY, detectInitialLanguage, interpolate, t, translate, useLanguageStore,
} from './index';

const leaves = (o: object, p = ''): string[] =>
  Object.entries(o).flatMap(([k, v]) => (typeof v === 'string' ? [p + k] : leaves(v, `${p}${k}.`))).sort();

beforeEach(() => {
  localStorage.clear();
  useLanguageStore.getState().setLang('en');
});

describe('i18n', () => {
  it('resolves nested keys per language', () => {
    expect(translate('en', 'common.back')).toBe('Back');
    expect(translate('es', 'common.back')).toBe('Atrás');
    expect(translate('pt', 'common.back')).toBe('Voltar');
  });

  it('interpolates named parameters and leaves unknown ones intact', () => {
    expect(interpolate('v{version} · {os}', { version: '1.0.3' })).toBe('v1.0.3 · {os}');
    expect(translate('en', 'appearance.storage', { path: '/data/vault.db' })).toBe('storage: /data/vault.db');
  });

  it('falls back to the key for unknown keys', () => {
    // @ts-expect-error unknown key is rejected at compile time
    expect(translate('es', 'nope.missing')).toBe('nope.missing');
  });

  it('keeps every locale in sync with English', () => {
    expect(leaves(es)).toEqual(leaves(en));
    expect(leaves(pt)).toEqual(leaves(en));
  });

  it('has no empty translations', () => {
    for (const dict of [en, es, pt]) {
      for (const key of leaves(dict)) {
        const value = key.split('.').reduce<unknown>((n, k) => (n as Record<string, unknown>)[k], dict);
        expect(value, key).not.toBe('');
      }
    }
  });

  it('persists the selected language and switches the non-hook translator', () => {
    useLanguageStore.getState().setLang('pt');
    expect(localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe('pt');
    expect(document.documentElement.lang).toBe('pt');
    expect(t('common.close')).toBe('Fechar');
    expect(detectInitialLanguage()).toBe('pt');
  });

  it('ignores invalid saved values', () => {
    localStorage.setItem(LANGUAGE_STORAGE_KEY, 'xx');
    expect(['en', 'es', 'pt']).toContain(detectInitialLanguage());
  });
});
