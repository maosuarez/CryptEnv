//! Secret-aware clipboard: copies made through `clipboard_write_secret` are
//! cleared 30 s later and at vault lock, but only while the clipboard still
//! holds what crypt-env put there (so a later user copy is never destroyed).
//!
//! Ownership is tracked with an opaque marker: the clipboard sequence number on
//! Windows, a SHA-256 of the text elsewhere (the text itself is never stored).
//! On Windows the copy is also flagged to stay out of clipboard history (Win+V)
//! and cloud clipboard sync.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use zeroize::Zeroizing;

/// How long a copied secret stays on the clipboard.
const CLEAR_AFTER: Duration = Duration::from_secs(30);

type Marker = Vec<u8>;

/// Platform clipboard operations; abstracted so the ownership logic is testable.
trait ClipboardBackend {
    /// Puts `text` on the clipboard and returns the marker identifying that copy.
    fn write_secret(&self, text: &str) -> Result<Marker, String>;
    /// Marker of the current clipboard content; `Ok(None)` when it is empty or
    /// not text. `Err` means the clipboard could not be inspected.
    fn current_marker(&self) -> Result<Option<Marker>, String>;
    fn clear(&self) -> Result<(), String>;
}

struct Pending {
    generation: u64,
    marker: Marker,
}

/// The latest secret copy only; a newer copy replaces the pending one.
static PENDING: Mutex<Option<Pending>> = Mutex::new(None);
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

fn lock_pending(slot: &Mutex<Option<Pending>>) -> std::sync::MutexGuard<'_, Option<Pending>> {
    // The guarded data is a plain marker; a poisoned lock is still consistent.
    slot.lock().unwrap_or_else(|e| e.into_inner())
}

/// Writes `text` and records ownership; returns the generation of this copy.
fn record_copy<B: ClipboardBackend>(
    backend: &B,
    slot: &Mutex<Option<Pending>>,
    text: &str,
) -> Result<u64, String> {
    let mut pending = lock_pending(slot);
    let marker = backend.write_secret(text)?;
    let generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
    *pending = Some(Pending { generation, marker });
    Ok(generation)
}

/// Clears the clipboard if it still holds our copy. With `only_generation`, a
/// copy that has since been replaced by a newer one is left to its own timer.
/// If the clipboard cannot be read (e.g. Wayland without focus) and no newer
/// copy of ours exists, it is cleared unconditionally (best effort).
fn clear_if_ours_impl<B: ClipboardBackend>(
    backend: &B,
    slot: &Mutex<Option<Pending>>,
    only_generation: Option<u64>,
) {
    let mut pending = lock_pending(slot);
    let Some(ours) = pending.as_ref() else { return };
    if only_generation.is_some_and(|g| g != ours.generation) {
        return;
    }
    let should_clear = match backend.current_marker() {
        Ok(Some(current)) => current == ours.marker,
        Ok(None) => false,
        Err(_) => true,
    };
    if should_clear {
        let _ = backend.clear();
    }
    *pending = None;
}

#[cfg(windows)]
mod platform {
    use super::{ClipboardBackend, Marker};
    use std::time::Duration;
    use windows::core::w;
    use windows::Win32::Foundation::{GlobalFree, HANDLE, HWND};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardSequenceNumber, OpenClipboard,
        RegisterClipboardFormatW, SetClipboardData,
    };
    use windows::Win32::System::Memory::{
        GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE,
    };

    const CF_UNICODETEXT: u32 = 13;

    pub struct Backend;

    pub fn backend() -> Option<Backend> {
        Some(Backend)
    }

    /// Runs `f` with the clipboard open; another process may hold it briefly.
    fn with_clipboard<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        let mut opened = false;
        for _ in 0..10 {
            // SAFETY: plain Win32 call; the clipboard is closed again below.
            if unsafe { OpenClipboard(HWND::default()) }.is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if !opened {
            return Err("clipboard unavailable".to_string());
        }
        let result = f();
        // SAFETY: the clipboard was opened above by this thread.
        let _ = unsafe { CloseClipboard() };
        result
    }

    /// Hands `bytes` to the (already open) clipboard under `format`.
    fn set_data(format: u32, bytes: &[u8]) -> Result<(), String> {
        // SAFETY: the allocation is locked, filled within its bounds and
        // unlocked; on success the clipboard owns it, otherwise it is freed.
        unsafe {
            let hmem = GlobalAlloc(GMEM_MOVEABLE, bytes.len())
                .map_err(|_| "clipboard allocation failed".to_string())?;
            let ptr = GlobalLock(hmem) as *mut u8;
            if ptr.is_null() {
                let _ = GlobalFree(hmem);
                return Err("clipboard allocation failed".to_string());
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
            let _ = GlobalUnlock(hmem);
            if SetClipboardData(format, HANDLE(hmem.0)).is_err() {
                let _ = GlobalFree(hmem);
                return Err("clipboard write failed".to_string());
            }
        }
        Ok(())
    }

    impl ClipboardBackend for Backend {
        fn write_secret(&self, text: &str) -> Result<Marker, String> {
            let mut utf16: Vec<u8> = text
                .encode_utf16()
                .chain(std::iter::once(0))
                .flat_map(u16::to_le_bytes)
                .collect();
            let result = with_clipboard(|| {
                // SAFETY: clipboard is open (with_clipboard).
                unsafe { EmptyClipboard() }.map_err(|_| "clipboard write failed".to_string())?;
                set_data(CF_UNICODETEXT, &utf16)?;
                // SAFETY: registering a named format has no preconditions.
                let (exclude, history, cloud) = unsafe {
                    (
                        RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing")),
                        RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory")),
                        RegisterClipboardFormatW(w!("CanUploadToCloudClipboard")),
                    )
                };
                let zero = 0u32.to_le_bytes();
                set_data(exclude, &zero)?;
                set_data(history, &zero)?;
                set_data(cloud, &zero)?;
                // SAFETY: no preconditions.
                Ok(unsafe { GetClipboardSequenceNumber() }.to_le_bytes().to_vec())
            });
            zeroize::Zeroize::zeroize(&mut utf16);
            result
        }

        fn current_marker(&self) -> Result<Option<Marker>, String> {
            // SAFETY: no preconditions.
            Ok(Some(unsafe { GetClipboardSequenceNumber() }.to_le_bytes().to_vec()))
        }

        fn clear(&self) -> Result<(), String> {
            with_clipboard(|| {
                // SAFETY: clipboard is open (with_clipboard).
                unsafe { EmptyClipboard() }.map_err(|_| "clipboard clear failed".to_string())
            })
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{ClipboardBackend, Marker};
    use sha2::{Digest, Sha256};
    use std::sync::OnceLock;
    use tauri::AppHandle;
    use tauri_plugin_clipboard_manager::ClipboardExt;

    /// Set on the first secret copy; `lock_vault` has no handle of its own.
    static APP: OnceLock<AppHandle> = OnceLock::new();

    pub struct Backend(AppHandle);

    pub fn init(app: &AppHandle) {
        let _ = APP.set(app.clone());
    }

    pub fn backend() -> Option<Backend> {
        APP.get().cloned().map(Backend)
    }

    fn hash(text: &str) -> Marker {
        Sha256::digest(text.as_bytes()).to_vec()
    }

    impl ClipboardBackend for Backend {
        fn write_secret(&self, text: &str) -> Result<Marker, String> {
            self.0
                .clipboard()
                .write_text(text.to_string())
                .map_err(|_| "clipboard write failed".to_string())?;
            Ok(hash(text))
        }

        fn current_marker(&self) -> Result<Option<Marker>, String> {
            match self.0.clipboard().read_text() {
                Ok(text) if text.is_empty() => Ok(None),
                Ok(text) => Ok(Some(hash(&text))),
                Err(_) => Err("clipboard unreadable".to_string()),
            }
        }

        fn clear(&self) -> Result<(), String> {
            self.0
                .clipboard()
                .clear()
                .map_err(|_| "clipboard clear failed".to_string())
        }
    }
}

/// Clears the clipboard if it still holds the last secret copied by crypt-env.
/// Called by `lock_vault`; a no-op when nothing is pending.
pub fn clear_if_ours() {
    if let Some(backend) = platform::backend() {
        clear_if_ours_impl(&backend, &PENDING, None);
    }
}

/// Copies a secret to the clipboard (history/cloud-excluded on Windows) and
/// schedules its removal after 30 s unless the clipboard changed meanwhile.
#[tauri::command]
pub async fn clipboard_write_secret(app: tauri::AppHandle, text: String) -> Result<(), String> {
    let text = Zeroizing::new(text);
    #[cfg(not(windows))]
    platform::init(&app);
    #[cfg(windows)]
    let _ = &app;

    let generation = tokio::task::spawn_blocking(move || {
        let backend = platform::backend().ok_or_else(|| "clipboard unavailable".to_string())?;
        record_copy(&backend, &PENDING, &text)
    })
    .await
    .map_err(|_| "clipboard task failed".to_string())??;

    tokio::spawn(async move {
        tokio::time::sleep(CLEAR_AFTER).await;
        let _ = tokio::task::spawn_blocking(move || {
            if let Some(backend) = platform::backend() {
                clear_if_ours_impl(&backend, &PENDING, Some(generation));
            }
        })
        .await;
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// In-memory clipboard whose marker is the content itself.
    struct Fake {
        content: RefCell<Option<String>>,
        unreadable: bool,
    }

    impl Fake {
        fn new() -> Self {
            Fake { content: RefCell::new(None), unreadable: false }
        }
        fn user_copy(&self, text: &str) {
            *self.content.borrow_mut() = Some(text.to_string());
        }
        fn marker_of(text: &str) -> Marker {
            text.as_bytes().to_vec()
        }
    }

    impl ClipboardBackend for Fake {
        fn write_secret(&self, text: &str) -> Result<Marker, String> {
            *self.content.borrow_mut() = Some(text.to_string());
            Ok(Fake::marker_of(text))
        }
        fn current_marker(&self) -> Result<Option<Marker>, String> {
            if self.unreadable {
                return Err("unreadable".into());
            }
            Ok(self.content.borrow().as_deref().map(Fake::marker_of))
        }
        fn clear(&self) -> Result<(), String> {
            *self.content.borrow_mut() = None;
            Ok(())
        }
    }

    fn slot() -> Mutex<Option<Pending>> {
        Mutex::new(None)
    }

    #[test]
    fn ours_is_cleared() {
        let (cb, s) = (Fake::new(), slot());
        let g = record_copy(&cb, &s, "hunter2").unwrap();
        clear_if_ours_impl(&cb, &s, Some(g));
        assert_eq!(*cb.content.borrow(), None);
        assert!(s.lock().unwrap().is_none());
    }

    #[test]
    fn replaced_content_is_untouched() {
        let (cb, s) = (Fake::new(), slot());
        let g = record_copy(&cb, &s, "hunter2").unwrap();
        cb.user_copy("unrelated");
        clear_if_ours_impl(&cb, &s, Some(g));
        assert_eq!(cb.content.borrow().as_deref(), Some("unrelated"));
    }

    #[test]
    fn stale_timer_does_not_clear_newer_copy() {
        let (cb, s) = (Fake::new(), slot());
        let old = record_copy(&cb, &s, "first").unwrap();
        record_copy(&cb, &s, "second").unwrap();
        clear_if_ours_impl(&cb, &s, Some(old));
        assert_eq!(cb.content.borrow().as_deref(), Some("second"));
        // Lock (no generation) clears the latest copy.
        clear_if_ours_impl(&cb, &s, None);
        assert_eq!(*cb.content.borrow(), None);
    }

    #[test]
    fn unreadable_clipboard_is_cleared_best_effort() {
        let (mut cb, s) = (Fake::new(), slot());
        record_copy(&cb, &s, "hunter2").unwrap();
        cb.unreadable = true;
        clear_if_ours_impl(&cb, &s, None);
        assert_eq!(*cb.content.borrow(), None);
    }

    #[test]
    fn nothing_pending_is_a_noop() {
        let (cb, s) = (Fake::new(), slot());
        cb.user_copy("mine");
        clear_if_ours_impl(&cb, &s, None);
        assert_eq!(cb.content.borrow().as_deref(), Some("mine"));
    }
}
