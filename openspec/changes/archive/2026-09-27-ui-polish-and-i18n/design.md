## Context

CryptEnv is a local-first secrets manager and environment variable coordinator built with Tauri 2.0, Rust, React 19, and Tailwind CSS. The desktop interface currently contains visual and informational legacy artifacts:
- Hardcoded metadata referencing an obsolete app name ("vault"), wrong version strings ("v2.0.0"), and inaccurate filesystem paths (`~/.vault/data.enc`).
- Hardcoded navigation back handlers routing to `vault` (Global Secrets) instead of `projects` (the primary landing view).
- Fixed dark palette with low contrast in secondary typography and no light theme option.
- Monolingual English interface with no localization infrastructure.

See [proposal.md](file:///home/maosuarez/Programas/crypt-env/openspec/changes/ui-polish-and-i18n/proposal.md) for full motivation and scope.

## Goals / Non-Goals

**Goals:**
- Provide dynamic, verified system diagnostics on LockScreen and Settings.
- Implement robust contextual back navigation through a navigation stack/history in `useVaultStore`.
- Introduce a Light Theme with clean industrial contrast alongside enhanced Dark Theme typography.
- Implement comprehensive internationalization supporting English (`en`), Spanish (`es`), and Portuguese (`pt`), configurable from Settings and persisted across sessions.

**Non-Goals:**
- Modifying underlying cryptographic implementations (`AES-256-GCM`, `Argon2id`, `sqlx`).
- Adding machine translation or dynamic network-fetched locale packs.
- Modifying CLI or MCP server output formats.

## Decisions

### 1. Dynamic System Diagnostics & Tauri Command
- **Decision**: Introduce a Tauri backend command `app_get_system_info` returning structured runtime metadata:
  ```rust
  pub struct AppSystemInfo {
      pub version: String,
      pub app_dir: String,
      pub db_path: String,
      pub os: String,
  }
  ```
- **Rationale**: Relying solely on client-side package JSON or hardcoded strings causes drift. Querying the Tauri `app.path().app_data_dir()` gives the exact filesystem path where the encrypted database lives on Windows, Linux, and macOS.
- **Alternatives Considered**:
  - Hardcoding path strings: Rejected as it caused the current bug (`~/.vault/data.enc`).
  - Using only `@tauri-apps/api/app`: Only provides version, not the resolved `app_data_dir` or verified crypto info.

### 2. Navigation History Stack in `useVaultStore`
- **Decision**: Extend `useVaultStore` with a navigation stack:
  ```typescript
  history: Screen[];
  go: (screen: Screen) => void;
  goBack: () => void;
  ```
  `go(screen)` appends the previous screen to `history`. `goBack()` pops the top screen from `history` and transitions to it. If `history` is empty or target equals current, it falls back to `'projects'`.
- **Rationale**: Solves the issue where Settings, Categories, and Edit items were hardcoded to route to `'vault'`. Now, opening Settings from Projects returns to Projects, while opening Settings from Global Secrets returns to Global Secrets.
- **Alternatives Considered**:
  - Hardcoding `'projects'` everywhere: Defective when user opens Settings or Edit from Global Secrets.

### 3. CSS Variable Theming & Contrast Enhancement
- **Decision**: Implement theme switching via `data-theme` attribute on the root document (`<html>`) mapped to semantic CSS variables in `src/index.css`:
  - Dark Theme: Increase contrast of `--color-tx3` (`#8896b3`) and `--color-tx4` (`#55617d`), ensuring WCAG AA (4.5:1) compliance on dark backgrounds.
  - Light Theme (`[data-theme="light"]`):
    - `--color-bg`: `#f4f6f8`
    - `--color-surface`: `#ffffff`
    - `--color-raised`: `#e9edf2`
    - `--color-bd`: `#cbd5e1`
    - `--color-bd2`: `#94a3b8`
    - `--color-tx`: `#0f172a`
    - `--color-tx2`: `#334155`
    - `--color-tx3`: `#475569`
    - `--color-tx4`: `#64748b`
    - Accent colors adjusted for light background visibility.
- **Persistence**: Saved in `localStorage` (`cryptenv_theme`) and applied synchronously in `index.html` to prevent flash of unstyled theme (FOUT).
- **Alternatives Considered**:
  - Pure Tailwind dark: variant classes everywhere: Tedious, error-prone, and bloats JSX. Semantic CSS variables in `src/index.css` keep component markup clean and single-sourced.

### 4. Lightweight Type-Safe i18n Architecture
- **Decision**: Create a modular TypeScript i18n subsystem in `src/i18n/` with typed dictionaries for `en`, `es`, and `pt`.
  - Stored in `localStorage` under `cryptenv_language`.
  - Accessible via a custom React hook `useTranslation()` / store.
  - Pluggable parameter interpolation (e.g. `t('settings.version', { version })`).
- **Rationale**: Adding heavy third-party i18n frameworks like `next-intl` or full `i18next` bundles brings unnecessary complexity and bundle size for a three-language desktop application. A typed dictionary approach guarantees compile-time completeness of missing keys across languages.
- **Alternatives Considered**:
  - `react-i18next`: Viable, but introduces external runtime dependencies. A lightweight typed hook provides zero overhead, zero external dependencies, and strict TypeScript key validation.

## Security & Threat Model

- **Zero Plaintext Secrets**: Translations and UI themes operate purely on UI strings and presentation tokens. No secret values, item payloads, or cryptographic tokens are touched or logged.
- **Diagnostic Safety**: The `app_get_system_info` command only exposes application version and directory paths (`app_dir`), never master passwords, keys, or plaintext contents.
- **Localhost & IPC Boundary**: All frontend-backend communication continues strictly through Tauri's internal IPC invoke handlers. No external network endpoints are used.

## Risks / Trade-offs

- [Risk]: Text expansion in Spanish and Portuguese causing layout overflow in fixed-width containers.
  → Mitigation: Test all screens with Spanish and Portuguese strings, ensuring responsive flex wraps, truncate/ellipsis where appropriate, and flexible button paddings.
- [Risk]: Contrast in Light Mode washing out custom brand colors.
  → Mitigation: Calibrate oklch accent and badge colors (`accent`, `danger`, `warn`, `cred`, `lnk`) specifically for light backgrounds.
- [Risk]: Navigation history cycle if user navigates back and forth.
  → Mitigation: Filter adjacent duplicate entries and cap history stack size to 20 entries.
