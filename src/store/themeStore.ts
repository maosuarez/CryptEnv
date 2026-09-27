import { create } from 'zustand';

export type Theme = 'dark' | 'light';

/** localStorage key; also read by public/theme-init.js before first paint. */
export const THEME_STORAGE_KEY = 'cryptenv_theme';

function readStoredTheme(): Theme {
  try {
    return localStorage.getItem(THEME_STORAGE_KEY) === 'light' ? 'light' : 'dark';
  } catch {
    return 'dark';
  }
}

/** Dark is the default palette in index.css; only light needs the attribute
 *  override, but it is always set so CSS can target either explicitly. */
export function applyTheme(theme: Theme) {
  document.documentElement.dataset.theme = theme;
}

interface ThemeStore {
  theme:    Theme;
  setTheme: (theme: Theme) => void;
}

export const useThemeStore = create<ThemeStore>((set) => ({
  theme: readStoredTheme(),
  setTheme: (theme) => {
    try {
      localStorage.setItem(THEME_STORAGE_KEY, theme);
    } catch {}
    applyTheme(theme);
    set({ theme });
  },
}));

applyTheme(useThemeStore.getState().theme);
