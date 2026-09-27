// Applies the saved theme before first paint to avoid a dark→light flash.
// Kept as an external file because the CSP forbids inline scripts.
// Must stay in sync with src/store/themeStore.ts (key `cryptenv_theme`).
(function () {
  try {
    document.documentElement.dataset.theme =
      localStorage.getItem('cryptenv_theme') === 'light' ? 'light' : 'dark';
  } catch (e) {}
})();
