import { invoke } from '@tauri-apps/api/core';
import { writeText } from '@tauri-apps/plugin-clipboard-manager';
import { useVaultStore } from '../store';
import { t } from '../i18n';

/** Copies a secret (value, passphrase, token) through the backend, which keeps
 *  it out of Windows clipboard history and clears it after 30 s or at lock.
 *  Shows a toast (`label`, default "Copied") noting the 30 s lifetime. */
export async function copySecret(text: string, label: string = t('common.copied')): Promise<void> {
  await invoke('clipboard_write_secret', { text });
  useVaultStore.getState().showToast(t('clipboard.secretCopied', { label }));
}

/** Copies non-secret text (project names, SQL, config snippets). The ONLY
 *  allowed use of the plugin's `writeText`; a guard test enforces it. */
export async function copyPlain(text: string): Promise<void> {
  await writeText(text);
}
