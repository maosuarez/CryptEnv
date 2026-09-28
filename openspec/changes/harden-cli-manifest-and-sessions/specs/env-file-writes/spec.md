## Purpose

Defines the safety invariants for every file crypt-env writes into a project tree (`.env` targets, `.env.example`, `.crypt-env.yaml`). These writes contain plaintext secrets or control where secrets get routed, so a hostile repository must not be able to redirect them.

## ADDED Requirements

### Requirement: Relative target paths stay inside the project root

A configured environment path that is relative MUST resolve inside the project root after all symlinks are resolved. Resolution SHALL canonicalize the project root and the target's nearest existing ancestor directory, and MUST reject the path if that canonical ancestor is outside the canonical root. `..` components, drive or stream specifiers (`:`), NUL and control characters MUST be rejected. A rejected path SHALL fail the injection for that environment with an error naming the configured path, and no secret SHALL be written anywhere for it.

#### Scenario: Symlinked directory escapes the root
- **WHEN** the project root contains `config -> /tmp/shared` and an environment targets `config/.env`
- **THEN** injection fails with a "must stay inside the project root" error and nothing is written to `/tmp/shared`

#### Scenario: Normal nested path
- **WHEN** an environment targets `apps/api/.env` and `apps/api` is a real directory inside the root
- **THEN** the file is written at `<root>/apps/api/.env`

### Requirement: Env-file writes never follow a final-component symlink

No `.env` target write and no `.env.example` write SHALL follow a symlink or reparse point at the final path component. This applies to relative and to absolute configured paths. If the target is a symlink, crypt-env MUST refuse the write with an error that names the path, and MUST leave both the link and the file it points to unmodified. The check and the open SHALL be race-free on Unix (open with no-follow semantics). On Windows the check SHALL be made immediately before opening, and reparse points SHALL be treated as symlinks.

#### Scenario: Planted .env symlink
- **WHEN** a cloned repository contains `.env -> /tmp/leak` and the user runs `crypt-env fill`
- **THEN** the fill reports an error for `.env`, `/tmp/leak` receives no data, and the symlink is unchanged

#### Scenario: Absolute path that is a symlink
- **WHEN** an environment is configured with the absolute path `/srv/app/.env`, which is a symlink
- **THEN** the injection refuses to write it and reports the path

### Requirement: Atomic, symlink-safe manifest writes

Writing `.crypt-env.yaml` (from `init`, a `config` pull, or the GUI's write-manifest action) MUST go through a uniquely named temporary file in the same directory. That file MUST be created exclusively (it fails if anything already exists at its path, including a symlink), fully written and flushed, and then renamed over `.crypt-env.yaml`. A pre-existing file or symlink with any fixed temporary name MUST NOT be opened, followed or modified. On failure the temporary file SHALL be removed, and any existing `.crypt-env.yaml` SHALL remain intact.

#### Scenario: Planted temp symlink
- **WHEN** a repository ships `.crypt-env.yaml.tmp -> ~/.bashrc` and the user runs `crypt-env init` or `config` (pull)
- **THEN** `~/.bashrc` is unchanged and `.crypt-env.yaml` is written correctly

#### Scenario: Write failure
- **WHEN** writing the temporary file fails partway (for example, the disk is full)
- **THEN** the previous `.crypt-env.yaml` is intact and no temporary file remains

### Requirement: Filesystem I/O does not block the vault

GUI and REST operations that touch a project directory (checking a root, writing a manifest) MUST NOT hold the vault's shared lock while doing filesystem I/O, and MUST run that I/O off the async runtime's worker threads. A slow or unreachable project root, such as a `\\wsl.localhost` path to a stopped distribution, SHALL delay only the operation that touches it.

#### Scenario: Stalled WSL root
- **WHEN** the GUI writes `.crypt-env.yaml` into a `\\wsl.localhost\…` root whose distribution is not responding
- **THEN** other GUI commands and REST requests that need the vault keep responding while that write is pending
