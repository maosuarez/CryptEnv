//! Global "toggle window" shortcut: parsing/validation, runtime
//! (re-)registration, and a pause flag used while the Settings screen is
//! recording a new combination.
//!
//! The app registers exactly one global shortcut, so re-registration uses
//! `unregister_all` + `on_shortcut` and remembers the active string in
//! [`HotkeyState`] so a failed swap can roll back to the previous one.

use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

/// Emitted (payload: active hotkey string) when the registered shortcut is
/// pressed while paused, i.e. while Settings is recording.
pub const HOTKEY_CAPTURED_EVENT: &str = "hotkey_captured";

/// Platform default: `Cmd+Alt+Z` on macOS, `Ctrl+Alt+Z` elsewhere.
pub fn default_hotkey() -> &'static str {
    if cfg!(target_os = "macos") {
        "Cmd+Alt+Z"
    } else {
        "Ctrl+Alt+Z"
    }
}

/// In-memory hotkey state, managed by Tauri.
///
/// `paused` is checked inside the shortcut callback instead of unregistering
/// the shortcut, so a webview crash mid-recording can never leave the app
/// without a registered hotkey.
#[derive(Default)]
pub struct HotkeyState {
    paused: AtomicBool,
    current: Mutex<Option<String>>,
}

impl HotkeyState {
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::SeqCst);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    pub fn current(&self) -> Option<String> {
        self.current.lock().ok().and_then(|c| c.clone())
    }

    fn set_current(&self, hotkey: &str) {
        if let Ok(mut c) = self.current.lock() {
            *c = Some(hotkey.to_string());
        }
    }
}

/// Canonicalise legacy tokens written by older frontends (`Meta` is not
/// understood by the shortcut parser; it maps to `Super`/`Cmd`).
pub fn normalize_hotkey(hotkey: &str) -> String {
    hotkey
        .split('+')
        .map(|t| {
            let t = t.trim();
            if t.eq_ignore_ascii_case("meta") {
                "Super"
            } else {
                t
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// Parses a hotkey string and requires at least one of Ctrl / Alt / Cmd so the
/// app never grabs plain typing keys (or Shift+key) system-wide.
pub fn parse_hotkey(hotkey: &str) -> Result<Shortcut, String> {
    let normalized = normalize_hotkey(hotkey);
    let shortcut = Shortcut::from_str(&normalized)
        .map_err(|e| format!("invalid hotkey '{normalized}': {e}"))?;
    if !shortcut
        .mods
        .intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER)
    {
        return Err(format!(
            "invalid hotkey '{normalized}': requires a Ctrl, Alt or Cmd modifier"
        ));
    }
    Ok(shortcut)
}

fn toggle_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let visible = window.is_visible().unwrap_or(false);
        if visible {
            let _ = window.hide();
        } else {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}

/// Registers `hotkey` as the (only) global window-toggle shortcut.
pub fn register_app_hotkey(app: &tauri::AppHandle, hotkey: &str) -> Result<(), String> {
    let shortcut = parse_hotkey(hotkey)?;
    app.global_shortcut()
        .on_shortcut(shortcut, |app, _shortcut, event| {
            if event.state() != ShortcutState::Pressed {
                return;
            }
            let state = app.state::<HotkeyState>();
            if state.is_paused() {
                // The OS consumes a registered combination before the webview
                // sees it, so hand it to the recorder explicitly.
                if let Some(current) = state.current() {
                    let _ = app.emit(HOTKEY_CAPTURED_EVENT, current);
                }
                return;
            }
            toggle_main_window(app);
        })
        .map_err(|e| format!("failed to register hotkey '{hotkey}': {e}"))?;
    app.state::<HotkeyState>().set_current(&normalize_hotkey(hotkey));
    Ok(())
}

/// Swaps the registered shortcut for `hotkey`. No-op when it is already the
/// active one; on registration failure the previous shortcut is restored.
pub fn replace_app_hotkey(app: &tauri::AppHandle, hotkey: &str) -> Result<(), String> {
    parse_hotkey(hotkey)?;
    let previous = app.state::<HotkeyState>().current();
    if previous.as_deref() == Some(normalize_hotkey(hotkey).as_str()) {
        return Ok(());
    }
    app.global_shortcut()
        .unregister_all()
        .map_err(|e| format!("failed to unregister hotkey: {e}"))?;
    if let Err(e) = register_app_hotkey(app, hotkey) {
        if let Some(prev) = previous {
            let _ = register_app_hotkey(app, &prev);
        }
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_hotkey_is_valid() {
        assert!(parse_hotkey(default_hotkey()).is_ok());
    }

    #[test]
    fn accepts_modifier_combinations() {
        for hk in ["Ctrl+Alt+Z", "Cmd+Shift+K", "Alt+F5", "Ctrl+Shift+ArrowUp", "Super+1"] {
            assert!(parse_hotkey(hk).is_ok(), "{hk} should be valid");
        }
    }

    #[test]
    fn rejects_keys_without_ctrl_alt_or_cmd() {
        for hk in ["A", "1", "Shift+A", "F5"] {
            assert!(parse_hotkey(hk).is_err(), "{hk} should be rejected");
        }
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_hotkey("Ctrl+").is_err());
        assert!(parse_hotkey("Ctrl+NotAKey").is_err());
    }

    #[test]
    fn legacy_meta_token_is_normalized() {
        assert_eq!(normalize_hotkey("Ctrl+Meta+K"), "Ctrl+Super+K");
        assert!(parse_hotkey("Meta+Alt+K").is_ok());
    }
}
