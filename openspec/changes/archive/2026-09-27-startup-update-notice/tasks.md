## 1. Backend

- [x] 1.1 Replace `lock().unwrap()` in `check_for_update` and `install_update` (`src-tauri/src/lib.rs`) with `map_err` to a `String` error. Verify `cargo check` and `cargo clippy --all-targets` pass and `grep -n "lock().unwrap()" src-tauri/src/lib.rs` has no hits.

## 2. Frontend

- [x] 2.1 Add `src/store/updateStore.ts` (zustand): `checked`, `version`, `dismissed`, `installing`, `installed`, `error`; actions `checkOnce()`, `install()`, `dismiss()`. `checkOnce` is a no-op after the first call and swallows errors.
- [x] 2.2 Add `src/components/ui/UpdateNotice.tsx`: renders only when `version && !dismissed` and the screen is not `lock`; INSTALL / LATER actions; installing, installed and error states. Tailwind tokens only.
- [x] 2.3 In `src/App.tsx`, call `checkOnce()` when `screen` first leaves `'lock'`, and mount `<UpdateNotice />` with the global overlays.
- [x] 2.4 Vitest: `checkOnce` calls `check_for_update` once across repeated unlocks; failure shows nothing; notice shows version; LATER hides it; INSTALL calls `install_update` and shows success / error; hidden on lock screen.

## 3. Docs

- [x] 3.1 Add `docs/release.md` release checklist (signing key, version bump in the three manifests, tag, workflow verification, end-to-end update test). Link it from `docs/building.md`.

- [x] 3.2 Add `scripts/verify-updater-sig.mjs` (minisign verification against the `tauri.conf.json` pubkey) and a release-job step that runs it on the three updater artifacts before publishing. Verify with a throwaway key: own pubkey → OK; real pubkey → key-ID mismatch; tampered file and empty `.sig` → FAIL.
- [x] 3.3 `release.yml`: resolve `RELEASE_TAG` from `inputs.tag || github.ref_name`; check out that ref in every job; use it for `latest.json`, `tag_name`, release name and pre-release detection.

## 4. Verification

- [x] 4.1 `cd src-tauri && cargo test` and `pnpm test` green.
- [x] 4.2 `pnpm exec tsc --noEmit` clean.
- [x] 4.3 `openspec validate startup-update-notice --strict` passes.
- [ ] 4.4 Manual (after the 1.0.2 release): build this branch locally with `version` temporarily lowered to `1.0.1` in `tauri.conf.json`, launch, unlock, confirm the notice announces 1.0.2 and INSTALL upgrades. Revert the version change afterwards.
