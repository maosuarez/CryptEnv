import { beforeEach, describe, expect, it } from 'vitest';
import { THEME_STORAGE_KEY, useThemeStore } from './themeStore';

beforeEach(() => {
  localStorage.clear();
  useThemeStore.getState().setTheme('dark');
});

describe('themeStore', () => {
  it('defaults to dark and marks the document root', () => {
    expect(useThemeStore.getState().theme).toBe('dark');
    expect(document.documentElement.dataset.theme).toBe('dark');
  });

  it('switches to light, updates the root attribute and persists', () => {
    useThemeStore.getState().setTheme('light');
    expect(useThemeStore.getState().theme).toBe('light');
    expect(document.documentElement.dataset.theme).toBe('light');
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe('light');
  });
});
