# Building & Distributing CryptEnv

This guide explains how to compile the three CryptEnv binaries and distribute them on GitHub for Windows, macOS, and Linux.

For cutting a public release (signing, tagging, updater verification) see [release.md](release.md).

CryptEnv produces three separate outputs:

1. **Desktop App** (`tauri-crypt-env.exe` / `.app` / `.deb`) — the GUI vault with hotkey support
2. **CLI binary** (`crypt-env.exe` / `crypt-env`) — terminal tool that connects to the running desktop app
3. **MCP server** (`crypt-env-mcp.exe` / `crypt-env-mcp`) — stdio MCP server for AI agents

---

## Prerequisites

### Windows (MSVC)

1. **Rust (MSVC toolchain)**
   ```powershell
   rustup toolchain install stable-msvc
   rustup default stable-msvc
   ```

2. **Visual Studio Build Tools 2022**
   - Download from https://visualstudio.microsoft.com/downloads/
   - Install "Desktop development with C++" workload
   - Required for MSVC compiler and Windows SDK

3. **Node.js LTS** — https://nodejs.org/
   ```powershell
   node --version  # v20.x or later
   ```

4. **pnpm**
   ```powershell
   npm install -g pnpm@latest
   ```

5. **WebView2 Runtime**
   - Pre-installed on Windows 11
   - For Windows 10: https://developer.microsoft.com/microsoft-edge/webview2/

### macOS

1. **Rust**
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   source $HOME/.cargo/env
   ```

2. **Xcode Command Line Tools**
   ```bash
   xcode-select --install
   ```

3. **Node.js LTS** — https://nodejs.org/

4. **pnpm**
   ```bash
   npm install -g pnpm@latest
   ```

### Linux (Ubuntu 20.04+, Debian 11+)

1. **Rust**
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   source $HOME/.cargo/env
   ```

2. **System dependencies**
   ```bash
   sudo apt update
   sudo apt install -y \
     libwebkit2gtk-4.1-dev \
     libgtk-3-dev \
     libayatana-appindicator3-dev \
     librsvg2-dev \
     build-essential \
     curl \
     wget \
     openssl \
     libssl-dev
   ```

3. **Node.js LTS** — https://nodejs.org/

4. **pnpm**
   ```bash
   npm install -g pnpm@latest
   ```

---

## Building the Desktop App

The Tauri desktop application is the primary CryptEnv interface. It includes an embedded webview, hotkey listener, and clipboard integration.

### Development Build

```bash
cd /path/to/crypt-env
pnpm install
pnpm tauri dev
```

This launches a development server with hot reload. The Rust backend and React frontend rebuild on file changes.

### Production Build

```bash
pnpm tauri build
```

**Windows installer with the CLI bundled** (what the release workflow runs):

```powershell
pnpm tauri:build:windows
```

`scripts/prepare-windows-bundle.mjs` builds `crypt-env` / `crypt-env-mcp` from the same revision, stages them as Tauri sidecars in `src-tauri/binaries/`, fails if `crypt-env --version` differs from the GUI version, and stages the static WSL helper at `src-tauri/resources/wsl/crypt-env-setup`; the build then merges `src-tauri/tauri.windows-bundle.conf.json`. The helper is a Linux binary, so build it from WSL first (the repo is shared over `\\wsl.localhost`):

```bash
cd src-tauri
rustup target add x86_64-unknown-linux-musl
cargo build --release -p crypt-env-setup --target x86_64-unknown-linux-musl
mkdir -p resources/wsl && cp "${CARGO_TARGET_DIR:-target}/x86_64-unknown-linux-musl/release/crypt-env-setup" resources/wsl/
```

(or point `CRYPTENV_WSL_HELPER` at an existing build). Plain `pnpm tauri build` / `pnpm tauri dev` and the macOS/Linux bundles are unaffected.

**Build output locations:**

**Windows:**
- Installer (NSIS): `src-tauri/target/release/bundle/nsis/*.exe`
- Windows Installer (MSI): `src-tauri/target/release/bundle/msi/*.msi`

**macOS:**
- Disk Image: `src-tauri/target/release/bundle/dmg/*.dmg`

**Linux:**
- Debian package: `src-tauri/target/release/bundle/deb/*.deb`
- RedHat package: `src-tauri/target/release/bundle/rpm/*.rpm`
- AppImage (universal): `src-tauri/target/release/bundle/appimage/*.AppImage`

### Build Time

First build: 5–15 minutes (depends on system and internet speed for dependencies).  
Subsequent builds: 1–3 minutes (incremental compilation).

---

## Building CLI and MCP Binaries

The CLI and MCP server are standalone Rust binaries. They do not include the GUI and can be compiled separately and quickly.

### Building CLI

```bash
cd src-tauri
cargo build --release --bin crypt-env
```

**Output:**
- Windows: `target/release/crypt-env.exe`
- macOS/Linux: `target/release/crypt-env`

### Building MCP Server

```bash
cd src-tauri
cargo build --release --bin crypt-env-mcp
```

**Output:**
- Windows: `target/release/crypt-env-mcp.exe`
- macOS/Linux: `target/release/crypt-env-mcp`

### Building Both

```bash
cd src-tauri
cargo build --release --bin crypt-env --bin crypt-env-mcp
```

Build time: 30 seconds – 2 minutes.

---

## Distribution on GitHub (Unsigned)

CryptEnv is distributed unsigned. This is suitable for open-source projects and developer tools where users trust the GitHub repository as the source of truth.

### Why unsigned?

- Code signing certificates cost $300–$500/year (EV Windows certs, Apple Developer account)
- Unsigned distributions work fine for open-source tools
- Users can verify integrity via SHA256 checksums published in releases
- SmartScreen and Gatekeeper warnings are minor friction, not a security issue

### Platform-Specific Considerations

#### Windows: SmartScreen Warnings

Unsigned executables trigger Windows SmartScreen (Windows Defender User Access Control) on first run.

**User experience:**
1. Double-click the installer (`.msi` or `.exe`)
2. SmartScreen appears: "Windows protected your PC"
3. User clicks "More info" → "Run anyway"
4. Installer proceeds normally

**To reduce friction:**

- Distribute the `.msi` (Windows Installer) rather than raw `.exe` — SmartScreen is slightly less aggressive
- Build reputation: each legitimate download reduces SmartScreen warnings (Microsoft's reputation system)
- Include SHA256 checksums in release notes so users can verify integrity
- Link to this GitHub repository in the installer description

**User bypass (PowerShell):**
```powershell
# Remove quarantine attribute before running
Unblock-File .\crypt-env-installer.exe
.\crypt-env-installer.exe
```

#### macOS: Gatekeeper Warnings

Unsigned `.app` bundles are quarantined by Gatekeeper on first launch.

**User experience:**
1. Open the downloaded `.dmg`
2. Drag `CryptEnv.app` to Applications
3. Launch CryptEnv
4. Gatekeeper blocks it: "CryptEnv cannot be opened because the developer cannot be verified"
5. User goes to System Preferences → Security & Privacy → "Open Anyway" (after first block)

**User bypass (Terminal):**
```bash
# Remove quarantine attribute
xattr -dr com.apple.quarantine /Applications/CryptEnv.app

# Then launch normally
open /Applications/CryptEnv.app
```

#### Linux

No signing required. Distribution via `.deb`, `.rpm`, or `.AppImage` is straightforward.

---

## GitHub Release Workflow

### Manual Release Process

1. **Build on each platform** (or use GitHub Actions — see below)

   ```bash
   # Windows
   pnpm tauri build

   # macOS
   pnpm tauri build

   # Linux
   pnpm tauri build
   ```

2. **Create a GitHub release**

   ```bash
   # Tag the release
   git tag v0.2.0
   git push origin v0.2.0

   # Or create via GitHub CLI
   gh release create v0.2.0 --draft
   ```

3. **Upload artifacts**

   ```bash
   # Upload desktop app installers
   gh release upload v0.2.0 \
     src-tauri/target/release/bundle/msi/*.msi \
     src-tauri/target/release/bundle/nsis/*.exe

   # Upload CLI and MCP binaries
   gh release upload v0.2.0 \
     src-tauri/target/release/crypt-env.exe \
     src-tauri/target/release/crypt-env-mcp.exe
   ```

4. **Generate and publish checksums**

   ```powershell
   # Windows PowerShell
   Get-FileHash src-tauri/target/release/bundle/msi/*.msi -Algorithm SHA256 | ForEach-Object { "$($_.Path)\t$($_.Hash)" }

   Get-FileHash src-tauri/target/release/crypt-env.exe -Algorithm SHA256 | ForEach-Object { "$($_.Path)\t$($_.Hash)" }
   ```

   ```bash
   # macOS/Linux
   sha256sum src-tauri/target/release/bundle/dmg/*.dmg > checksums.txt
   sha256sum src-tauri/target/release/crypt-env >> checksums.txt
   sha256sum src-tauri/target/release/crypt-env-mcp >> checksums.txt

   gh release upload v0.2.0 checksums.txt
   ```

5. **Publish the release**

   Update the release description with:
   - Summary of changes (from CHANGELOG.md)
   - SHA256 checksums for all artifacts
   - Installation instructions per platform
   - Known issues

### Automated GitHub Actions

Create `.github/workflows/release.yml` to build on every tag:

```yaml
name: Build & Release

on:
  push:
    tags:
      - 'v*'

jobs:
  build-windows:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust (MSVC)
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: x86_64-pc-windows-msvc

      - name: Setup Node.js
        uses: actions/setup-node@v4
        with:
          node-version: '20'

      - name: Install pnpm
        uses: pnpm/action-setup@v4
        with:
          version: 9

      - name: Install dependencies
        run: pnpm install

      - name: Build desktop app
        run: pnpm tauri build

      - name: Build CLI and MCP
        run: |
          cd src-tauri
          cargo build --release --bin crypt-env --bin crypt-env-mcp

      - name: Upload artifacts
        uses: actions/upload-artifact@v4
        with:
          name: windows-artifacts
          path: |
            src-tauri/target/release/bundle/msi/*.msi
            src-tauri/target/release/bundle/nsis/*.exe
            src-tauri/target/release/crypt-env.exe
            src-tauri/target/release/crypt-env-mcp.exe

  build-macos:
    runs-on: macos-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable

      - name: Setup Node.js
        uses: actions/setup-node@v4
        with:
          node-version: '20'

      - name: Install pnpm
        uses: pnpm/action-setup@v4
        with:
          version: 9

      - name: Install dependencies
        run: pnpm install

      - name: Build desktop app
        run: pnpm tauri build

      - name: Build CLI and MCP
        run: |
          cd src-tauri
          cargo build --release --bin crypt-env --bin crypt-env-mcp

      - name: Upload artifacts
        uses: actions/upload-artifact@v4
        with:
          name: macos-artifacts
          path: |
            src-tauri/target/release/bundle/dmg/*.dmg
            src-tauri/target/release/crypt-env
            src-tauri/target/release/crypt-env-mcp

  build-linux:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable

      - name: Install system dependencies
        run: |
          sudo apt-get update
          sudo apt-get install -y \
            libwebkit2gtk-4.1-dev \
            libgtk-3-dev \
            libayatana-appindicator3-dev \
            librsvg2-dev \
            build-essential

      - name: Setup Node.js
        uses: actions/setup-node@v4
        with:
          node-version: '20'

      - name: Install pnpm
        uses: pnpm/action-setup@v4
        with:
          version: 9

      - name: Install dependencies
        run: pnpm install

      - name: Build desktop app
        run: pnpm tauri build

      - name: Build CLI and MCP
        run: |
          cd src-tauri
          cargo build --release --bin crypt-env --bin crypt-env-mcp

      - name: Upload artifacts
        uses: actions/upload-artifact@v4
        with:
          name: linux-artifacts
          path: |
            src-tauri/target/release/bundle/deb/*.deb
            src-tauri/target/release/bundle/rpm/*.rpm
            src-tauri/target/release/bundle/appimage/*.AppImage
            src-tauri/target/release/crypt-env
            src-tauri/target/release/crypt-env-mcp

  release:
    needs: [build-windows, build-macos, build-linux]
    runs-on: ubuntu-latest
    permissions:
      contents: write
    if: startsWith(github.ref, 'refs/tags/')
    steps:
      - uses: actions/checkout@v4

      - name: Download all artifacts
        uses: actions/download-artifact@v4

      - name: Generate checksums
        run: |
          sha256sum windows-artifacts/* > checksums.txt
          sha256sum macos-artifacts/* >> checksums.txt
          sha256sum linux-artifacts/* >> checksums.txt

      - name: Create GitHub Release
        uses: softprops/action-gh-release@v2
        with:
          files: |
            windows-artifacts/**
            macos-artifacts/**
            linux-artifacts/**
            checksums.txt
          body: |
            See CHANGELOG.md for details on this release.

            ## Checksums
            Verify integrity with:
            ```bash
            sha256sum -c checksums.txt
            ```
```

To use this workflow:

1. Create `.github/workflows/release.yml` with the above content
2. Push a tag: `git tag v0.2.0 && git push origin v0.2.0`
3. GitHub Actions automatically builds on all platforms and creates a release

---

## Installing CLI and MCP from a Release

After building and distributing the binaries, users can install them as command-line tools.

### Windows

The NSIS installer already installs `crypt-env.exe` and `crypt-env-mcp.exe` next to the GUI and adds that folder to your user `PATH` — open a new terminal and run `crypt-env --version`. The manual steps below are only needed if you want the CLI without the GUI.

1. Download `crypt-env.exe` and `crypt-env-mcp.exe` from the GitHub Release
2. Add them to `%PATH%`:

   **Option A: System32 (requires admin)**
   ```powershell
   # Run PowerShell as Administrator
   Copy-Item .\crypt-env.exe -Destination "C:\Windows\System32\"
   Copy-Item .\crypt-env-mcp.exe -Destination "C:\Windows\System32\"
   ```

   **Option B: User PATH directory**
   ```powershell
   # Create a directory for local binaries (if it doesn't exist)
   mkdir $env:USERPROFILE\AppData\Local\Programs\crypt-env
   Copy-Item .\crypt-env.exe -Destination "$env:USERPROFILE\AppData\Local\Programs\crypt-env\"
   Copy-Item .\crypt-env-mcp.exe -Destination "$env:USERPROFILE\AppData\Local\Programs\crypt-env\"

   # Add to PATH (permanent)
   $path = [Environment]::GetEnvironmentVariable("PATH", "User")
   $newPath = "$path;$env:USERPROFILE\AppData\Local\Programs\crypt-env"
   [Environment]::SetEnvironmentVariable("PATH", $newPath, "User")
   ```

3. Verify installation:
   ```powershell
   crypt-env --help
   crypt-env-mcp --help
   ```

### macOS & Linux

1. Download `crypt-env` and `crypt-env-mcp` from the GitHub Release
2. Make executable and install:

   ```bash
   chmod +x crypt-env crypt-env-mcp
   sudo mv crypt-env /usr/local/bin/
   sudo mv crypt-env-mcp /usr/local/bin/
   ```

3. Verify installation:
   ```bash
   crypt-env --help
   crypt-env-mcp --help
   ```

---

## Troubleshooting

### Windows: Build fails with "Microsoft C++ Build Tools not found"

Install Visual Studio Build Tools 2022 from https://visualstudio.microsoft.com/downloads/. Choose "Desktop development with C++".

### macOS: Gatekeeper quarantine persists

If a user still sees the quarantine warning after your release, they can reset it:

```bash
xattr -dr com.apple.quarantine /Applications/CryptEnv.app
```

### Linux: Missing WebKit2GTK library

```bash
sudo apt install libwebkit2gtk-4.1-dev
```

If you're on Fedora/RHEL:
```bash
sudo dnf install webkit2gtk4-devel gtk3-devel
```

### Build intermittent failures

Sometimes builds fail due to network timeouts downloading dependencies. Retry:

```bash
cargo clean  # Optional: start fresh
pnpm tauri build
```

---

## Release Checklist

Before publishing a release:

- [ ] Update `CHANGELOG.md` with all changes since last release
- [ ] Update version in `src-tauri/Cargo.toml` and `src/package.json` if needed
- [ ] Test the build on each platform locally (or via Actions)
- [ ] Verify CLI and MCP binaries work (`--help` and `--version`)
- [ ] Windows: after installing the NSIS build, a new terminal's `crypt-env --version` equals the GUI version (also asserted at build time by `scripts/prepare-windows-bundle.mjs`), and uninstall removes the `PATH` entry
- [ ] Generate SHA256 checksums for all artifacts
- [ ] Create a GitHub Release with checksums in the description
- [ ] Test that users can download, unblock (Windows), and run the binaries
- [ ] Update the README if there are breaking changes or new features

---

## Next Steps

- For running CryptEnv in development, see the main [README.md](../README.md)
- For API documentation, see the REST API section in [README.md](../README.md)
- For security guidelines, see [SECURITY.md](../SECURITY.md)
