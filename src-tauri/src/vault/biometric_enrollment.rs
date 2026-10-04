//! Biometric (Windows Hello) enrollment lifecycle: enroll, invalidate on
//! password change or disable, legacy cleanup at startup and the re-enroll
//! notice. The unlock itself commits through `unlock::unlock_with_biometric`.

use std::sync::Arc;

use super::{unlock, SharedState};
use crate::biometric::{wrap, HelloSigner, BLOB_SETTING, NOTICE_SETTING};
use crate::db::VaultDb;

const CHANGED: &str = "the vault changed while enrolling, try again";

/// Enrolls Hello unlock for the unlocked vault after re-checking `password`.
/// Stores only the wrapped vault key and a challenge, never the password.
pub async fn enroll(
    shared: &SharedState,
    signer: Arc<dyn HelloSigner>,
    password: &[u8],
) -> Result<(), String> {
    let (vault_key, epoch) = {
        let s = shared.lock().await;
        let key = s.key.as_ref().ok_or("vault is locked")?;
        (key.clone(), s.epoch)
    };
    unlock::verify_password(shared, password).await?;

    let blob = tokio::task::spawn_blocking(move || wrap::enroll(&*signer, &vault_key))
        .await
        .map_err(|_| "biometric task failed".to_string())?
        .map_err(|e| e.message())?;

    // Same re-check as the unlock commit: a lock, reset or re-key while the
    // Hello prompts were open must not leave an enrollment for the old key.
    let s = shared.lock().await;
    if s.epoch != epoch || s.key.is_none() {
        return Err(CHANGED.to_string());
    }
    s.db.set_setting(BLOB_SETTING, &blob).await?;
    s.db.set_setting(NOTICE_SETTING, "").await
}

/// Removes the enrollment: the Hello credential (best effort) and the stored
/// blob. With `notify`, also flags that the user has to re-enroll. Without an
/// enrollment this does nothing, so no notice is raised for users who never
/// enrolled.
pub async fn invalidate(
    shared: &SharedState,
    signer: Arc<dyn HelloSigner>,
    notify: bool,
) -> Result<(), String> {
    let had_blob = {
        let s = shared.lock().await;
        let had = s.db.get_setting(BLOB_SETTING).await?.is_some_and(|v| !v.is_empty());
        if had {
            s.db.set_setting(BLOB_SETTING, "").await?;
            if notify {
                s.db.set_setting(NOTICE_SETTING, "1").await?;
            }
        }
        had
    };
    if had_blob {
        // The blob is already gone, which is what makes unlock impossible.
        let _ = tokio::task::spawn_blocking(move || signer.delete()).await;
    }
    Ok(())
}

/// Disabling deletes the credential and the blob even if the blob was already
/// missing (the credential may still exist), and raises no notice.
pub async fn disable(shared: &SharedState, signer: Arc<dyn HelloSigner>) -> Result<(), String> {
    {
        let s = shared.lock().await;
        s.db.set_setting(BLOB_SETTING, "").await?;
    }
    let _ = tokio::task::spawn_blocking(move || signer.delete()).await;
    Ok(())
}

/// Startup cleanup: a stored blob that is not the v2 format is the legacy
/// DPAPI-protected password. It is deleted (and the database compacted so the
/// old bytes do not linger) and the re-enroll notice is raised. Returns whether
/// a legacy blob was removed. Needs no unlock.
pub async fn remove_legacy_blob(db: &VaultDb) -> Result<bool, String> {
    let legacy = match db.get_setting(BLOB_SETTING).await? {
        Some(v) if !v.is_empty() => wrap::Enrollment::parse(&v).is_none(),
        _ => false,
    };
    if !legacy {
        return Ok(false);
    }
    db.set_setting(BLOB_SETTING, "").await?;
    db.set_setting(NOTICE_SETTING, "1").await?;
    db.compact().await?;
    Ok(true)
}

pub async fn notice_pending(shared: &SharedState) -> Result<bool, String> {
    let s = shared.lock().await;
    Ok(s.db.get_setting(NOTICE_SETTING).await?.as_deref() == Some("1"))
}

pub async fn dismiss_notice(shared: &SharedState) -> Result<(), String> {
    let s = shared.lock().await;
    s.db.set_setting(NOTICE_SETTING, "").await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::biometric::wrap::tests::FakeHello;
    use crate::test_support::unlocked_vault_empty;

    fn signer(h: FakeHello) -> (Arc<FakeHello>, Arc<dyn HelloSigner>) {
        let h = Arc::new(h);
        (h.clone(), h)
    }

    async fn setting(shared: &SharedState, key: &str) -> Option<String> {
        shared.lock().await.db.get_setting(key).await.unwrap()
    }

    #[tokio::test]
    async fn enroll_stores_v2_blob_without_password_or_key() {
        let v = unlocked_vault_empty().await;
        let (_, s) = signer(FakeHello::new());
        enroll(&v.state, s, v.master_password.as_bytes()).await.unwrap();

        let blob = setting(&v.state, BLOB_SETTING).await.unwrap();
        assert!(wrap::Enrollment::parse(&blob).is_some());
        assert!(!blob.contains(&v.master_password));
        let key_hex = crate::crypto::hex_encode(v.state.lock().await.key.as_ref().unwrap().expose());
        assert!(!blob.contains(&key_hex));
    }

    #[tokio::test]
    async fn enroll_refuses_wrong_password_and_non_deterministic_signer() {
        let v = unlocked_vault_empty().await;
        let (_, s) = signer(FakeHello::new());
        assert!(enroll(&v.state, s, b"not-the-password").await.is_err());

        let mut h = FakeHello::new();
        h.randomized = true;
        let (_, s) = signer(h);
        let err = enroll(&v.state, s, v.master_password.as_bytes()).await.unwrap_err();
        assert!(err.contains("not supported on this device"));
        assert!(setting(&v.state, BLOB_SETTING).await.is_none_or(|b| b.is_empty()));
    }

    #[tokio::test]
    async fn enroll_requires_unlocked_vault() {
        let v = unlocked_vault_empty().await;
        v.state.lock().await.set_key(None);
        let (_, s) = signer(FakeHello::new());
        assert!(enroll(&v.state, s, v.master_password.as_bytes()).await.is_err());
    }

    #[tokio::test]
    async fn biometric_unlock_round_trip_and_cancel() {
        let v = unlocked_vault_empty().await;
        let hello = Arc::new(FakeHello::new());
        enroll(&v.state, hello.clone(), v.master_password.as_bytes()).await.unwrap();
        let key = v.state.lock().await.key.as_ref().unwrap().clone();
        crate::vault::lock_vault(&v.state).await;
        assert!(v.state.lock().await.key.is_none());

        let out = unlock::unlock_with_biometric(&v.state, hello.clone(), false).await.unwrap();
        let s = v.state.lock().await;
        assert!(s.key.as_ref().unwrap().ct_eq(&key));
        assert_eq!(out.epoch, s.epoch);
        drop(s);

        crate::vault::lock_vault(&v.state).await;
        let mut cancel = FakeHello::new();
        cancel.cancel = true;
        let res = unlock::unlock_with_biometric(&v.state, Arc::new(cancel), false).await;
        assert!(res.is_err());
        assert!(v.state.lock().await.key.is_none());
    }

    #[tokio::test]
    async fn biometric_unlock_with_wrong_signature_stays_locked() {
        let v = unlocked_vault_empty().await;
        let hello = Arc::new(FakeHello::new());
        enroll(&v.state, hello.clone(), v.master_password.as_bytes()).await.unwrap();
        crate::vault::lock_vault(&v.state).await;
        *hello.secret.lock().unwrap() = 99;
        assert!(unlock::unlock_with_biometric(&v.state, hello, false).await.is_err());
        assert!(v.state.lock().await.key.is_none());
    }

    #[tokio::test]
    async fn password_change_invalidates_enrollment_and_sets_notice() {
        let v = unlocked_vault_empty().await;
        let (hello, s) = signer(FakeHello::new());
        enroll(&v.state, s.clone(), v.master_password.as_bytes()).await.unwrap();

        invalidate(&v.state, s, true).await.unwrap();

        assert_eq!(setting(&v.state, BLOB_SETTING).await.as_deref(), Some(""));
        assert!(notice_pending(&v.state).await.unwrap());
        assert_eq!(*hello.deletes.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn change_password_drops_enrollment() {
        let v = unlocked_vault_empty().await;
        let (hello, s) = signer(FakeHello::new());
        enroll(&v.state, s, v.master_password.as_bytes()).await.unwrap();

        unlock::change_password_with_signer(
            &v.state,
            &v.master_password,
            "a-brand-new-password",
            hello.clone(),
        )
        .await
        .unwrap();

        assert_eq!(setting(&v.state, BLOB_SETTING).await.as_deref(), Some(""));
        assert!(notice_pending(&v.state).await.unwrap());
        assert_eq!(*hello.deletes.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn invalidate_without_enrollment_raises_no_notice() {
        let v = unlocked_vault_empty().await;
        let (hello, s) = signer(FakeHello::new());
        invalidate(&v.state, s, true).await.unwrap();
        assert!(!notice_pending(&v.state).await.unwrap());
        assert_eq!(*hello.deletes.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn disable_deletes_credential_and_blob_without_notice() {
        let v = unlocked_vault_empty().await;
        let (hello, s) = signer(FakeHello::new());
        enroll(&v.state, s.clone(), v.master_password.as_bytes()).await.unwrap();
        disable(&v.state, s).await.unwrap();
        assert_eq!(setting(&v.state, BLOB_SETTING).await.as_deref(), Some(""));
        assert!(!notice_pending(&v.state).await.unwrap());
        assert_eq!(*hello.deletes.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn legacy_blob_is_removed_and_notice_set() {
        let v = unlocked_vault_empty().await;
        {
            let s = v.state.lock().await;
            // v1 shape: hex of a DPAPI blob.
            s.db.set_setting(BLOB_SETTING, "01000000d08c9ddf0115d1118c7a00c04fc297eb").await.unwrap();
        }
        let removed = remove_legacy_blob(&v.state.lock().await.db).await.unwrap();
        assert!(removed);
        assert_eq!(setting(&v.state, BLOB_SETTING).await.as_deref(), Some(""));
        assert!(notice_pending(&v.state).await.unwrap());

        // A second run finds nothing and does not re-raise a dismissed notice.
        dismiss_notice(&v.state).await.unwrap();
        assert!(!remove_legacy_blob(&v.state.lock().await.db).await.unwrap());
        assert!(!notice_pending(&v.state).await.unwrap());
    }

    #[tokio::test]
    async fn valid_v2_blob_survives_startup_cleanup() {
        let v = unlocked_vault_empty().await;
        let (_, s) = signer(FakeHello::new());
        enroll(&v.state, s, v.master_password.as_bytes()).await.unwrap();
        let before = setting(&v.state, BLOB_SETTING).await;
        assert!(!remove_legacy_blob(&v.state.lock().await.db).await.unwrap());
        assert_eq!(setting(&v.state, BLOB_SETTING).await, before);
        assert!(!notice_pending(&v.state).await.unwrap());
    }
}
