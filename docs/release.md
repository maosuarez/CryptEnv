# Release Checklist

How to cut a CryptEnv release so the in-app updater picks it up. Releases are built by `.github/workflows/release.yml` on a `vX.Y.Z` tag push.

## How updates reach users

1. Installed apps fetch `https://github.com/maosuarez/crypt-env/releases/latest/download/latest.json` (`src-tauri/tauri.conf.json` → `plugins.updater.endpoints`).
2. If `version` there is newer, the app shows the update notice after the first unlock (or on **Settings → Check for updates**).
3. INSTALL downloads the platform artifact and verifies it against the minisign `pubkey` embedded in the installed app. Signature mismatch → install refused.

The signing key was rotated in v1.0.2 (the original private key was lost). Installs from v1.0.2 onwards trust key ID `B6ED23EC84C1565B`; older installs trust the retired key and can never auto-update.

## 1. Before tagging

- [ ] **Signing key matches the pubkey.** Installed apps only trust the `pubkey` in `tauri.conf.json` (key ID `B6ED23EC84C1565B`). A different key in the `TAURI_SIGNING_PRIVATE_KEY` secret still builds and signs fine, but every client rejects the update. The release workflow now blocks this (see §3). To check a key locally before uploading it:

  ```bash
  echo test > /tmp/probe.bin
  pnpm tauri signer sign -f ~/.tauri/cryptenv-updater.key -p "" /tmp/probe.bin
  node scripts/verify-updater-sig.mjs /tmp/probe.bin
  # OK   /tmp/probe.bin                                 → this key matches
  # FAIL ... signed with key XXXX, but ... trusts B6ED23EC84C1565B → wrong key
  ```

  Then upload it (value = the key file content as-is, no password):

  ```bash
  gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/cryptenv-updater.key
  ```

  If the private key is lost, installed clients can never auto-update again. Keep an offline backup. Rotating to a new key means shipping one release manually (download + install) that embeds the new pubkey.
- [ ] Bump the version to the same value in **all** of the following. `scripts/bump-version.ps1` covers the first three, but not the crate.
  - `package.json`
  - `src-tauri/tauri.conf.json`
  - `src-tauri/Cargo.toml` (`[package] version`)
  - `src-tauri/crates/crypt-env-setup/Cargo.toml`
  - then `cd src-tauri && cargo check` so `Cargo.lock` updates.
- [ ] `cd src-tauri && cargo test` and `pnpm test` green; `pnpm exec tsc --noEmit` clean.
- [ ] Changes merged to `main`; OpenSpec changes for this release archived (`openspec list`).

## 2. Tag and release

```bash
git checkout main && git pull
git tag vX.Y.Z
git push origin vX.Y.Z
```

- Pushing the tag triggers the workflow. To re-run a release, use **Run workflow** with `tag: vX.Y.Z`. Every job checks out that tag and publishes to it (`RELEASE_TAG`), not the branch the run was dispatched from.
- The tag filter is `v[0-9]+.[0-9]+.[0-9]+`, so pre-release tags (`v1.1.0-rc1`) only run through **Run workflow**, and they're published as pre-releases (never served by `releases/latest`).

## 3. Verify the workflow output

- [ ] All jobs green, including **Verify updater signatures are present** and **Verify updater signatures match tauri.conf.json pubkey**. The second step fails before the release is created if the secret doesn't match the pubkey.
- [ ] Release assets include `CryptEnv_X.Y.Z_x64-setup.exe` + `.sig`, `CryptEnv.app.tar.gz` + `.sig`, `CryptEnv_X.Y.Z_amd64.AppImage` + `.sig`, and `latest.json`.
- [ ] The release is marked **Latest** (not draft or pre-release). `releases/latest` ignores drafts and pre-releases.
- [ ] The public endpoint serves the new manifest with non-empty signatures:

  ```bash
  curl -sL https://github.com/maosuarez/crypt-env/releases/latest/download/latest.json \
    | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d["version"]); [print(p, bool(v["signature"])) for p,v in d["platforms"].items()]'
  ```

## 4. End-to-end update test

- [ ] On Windows, launch an **older** install (the previous release, or a local build with `version` temporarily lowered in `tauri.conf.json`), unlock the vault, and check that the notice shows `Update available: vX.Y.Z`.
- [ ] Click INSTALL. The NSIS installer runs, the app relaunches, and **Settings** shows the new version.
- [ ] Optional: repeat on macOS (Apple Silicon) / Linux AppImage. On these the app must be restarted manually after install.

## Known limitations

- `latest.json` has entries only for `windows-x86_64`, `darwin-aarch64` and `linux-x86_64` (AppImage). Intel Macs and `.deb` installs don't auto-update; they need a manual download.
- v1.0.1 and earlier embed the retired pubkey and never received valid signatures, so they can't update in-app. Users need to download and install v1.0.2 once; from v1.0.2 on, updates and the startup notice work automatically.
