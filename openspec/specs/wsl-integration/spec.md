# WSL Integration Specification

## Purpose

The Windows-only capability by which the CryptEnv GUI detects a WSL environment, reports its readiness for the "vault on Windows, CLI in WSL" workflow, and configures or removes the `crypt-env` client inside a chosen distribution without destroying existing shell content.

## Requirements

### Requirement: WSL integration is Windows-only and detection-gated

The WSL Integration settings section SHALL be shown only when the application is running on Windows and at least one WSL distribution is detected. On any non-Windows platform the section SHALL NOT render, and the WSL Tauri commands SHALL return a typed "unsupported platform" error without side effects.

#### Scenario: Hidden on non-Windows

- **WHEN** the app runs on macOS or Linux
- **THEN** the WSL Integration section is absent from Settings
- **AND** invoking any WSL command returns an unsupported-platform error and touches no files

#### Scenario: Hidden on Windows without WSL

- **WHEN** the app runs on Windows and no WSL distribution is installed
- **THEN** the WSL Integration section is not shown (or is shown only as an inert "WSL not detected" note)

#### Scenario: Shown on Windows with WSL

- **WHEN** the app runs on Windows and one or more distributions are detected
- **THEN** the WSL Integration section renders with the list of distributions

### Requirement: WSL environment detection

The GUI SHALL provide a detection operation that reports: whether WSL is available; for each distribution its name, its default Linux user, and whether the cryptenv client configuration is already installed in that distribution's shell startup; and whether `networkingMode=mirrored` is present in `%USERPROFILE%\.wslconfig`. Distribution names SHALL be obtained from the system WSL tooling and its output decoded correctly (UTF-16LE).

#### Scenario: Enumerate distributions

- **WHEN** the detection operation runs on Windows with two installed distributions
- **THEN** it returns both distribution names, each with its resolved default user and an installed/not-installed flag

#### Scenario: Report mirrored-networking status

- **WHEN** `%USERPROFILE%\.wslconfig` contains `networkingMode=mirrored` under `[wsl2]`
- **THEN** the detection result marks mirrored networking as enabled
- **AND** when the file is absent or lacks that setting, it is marked as not enabled

#### Scenario: Detection is read-only

- **WHEN** the detection operation runs
- **THEN** it modifies no files inside any distribution and does not change `.wslconfig`

#### Scenario: Garbled tooling output does not crash

- **WHEN** the system WSL tooling returns unexpected or non-UTF-16LE bytes
- **THEN** the operation returns a typed error and the section shows a non-fatal "could not detect" state

### Requirement: One-click client configuration for a distribution

The GUI SHALL provide an operation that configures the `crypt-env` client inside a named, detected distribution by performing the non-destructive shell configuration defined by the `cli` capability's `setup wsl` contract. It SHALL target a distribution only if that name appears in the current detection list. It SHALL report back the concrete actions taken: the managed env file created, the rc files touched, the marker block inserted, and the backup path. It MUST NOT perform destructive edits to shell files and MUST NOT copy the TLS certificate into the distribution.

#### Scenario: Configure a distribution

- **WHEN** the user selects a detected distribution and confirms "Configure"
- **THEN** the client configuration is applied inside that distribution using the same marker-block/backup/`env.sh` rules as `crypt-env setup wsl`
- **AND** the GUI shows which files were created or appended to and where the backup was written

#### Scenario: Configuration references the live certificate

- **WHEN** configuration is applied
- **THEN** the generated env file points at the Windows certificate via its `/mnt/c` path
- **AND** no copy of the certificate is written inside the distribution

#### Scenario: Re-configuring is non-destructive

- **WHEN** "Configure" is run again for a distribution that already has the marker block
- **THEN** only the managed env file is refreshed and the rc files are left unchanged
- **AND** no second backup is created

#### Scenario: Reject an unknown distribution

- **WHEN** a configure request names a distribution not present in the latest detection list
- **THEN** the operation refuses with a typed error and makes no changes

#### Scenario: External tooling is invoked safely

- **WHEN** the operation runs the system WSL tooling
- **THEN** the distribution name is passed as a discrete argument, never interpolated into a shell command string

### Requirement: Reversible client configuration

The GUI SHALL provide an operation that removes the cryptenv client configuration from a named distribution, reversing exactly what the configuration operation added (marker block and managed env file) and leaving all other shell content intact. Removing when nothing is installed SHALL succeed as a no-op.

#### Scenario: Remove configuration

- **WHEN** the user chooses "Remove" for a configured distribution
- **THEN** the marker block and the managed env file are deleted inside that distribution
- **AND** every other line of the shell rc files is preserved

#### Scenario: Remove when absent

- **WHEN** "Remove" is chosen for a distribution with no cryptenv configuration
- **THEN** the operation completes successfully without changing any file

### Requirement: Mirrored-networking guidance without automatic edits

When mirrored networking is not enabled, the GUI SHALL display the exact `.wslconfig` snippet to add and explain that applying it requires `wsl --shutdown` and affects all WSL networking. The GUI MUST NOT modify `%USERPROFILE%\.wslconfig` on the user's behalf.

#### Scenario: Show guidance when disabled

- **WHEN** detection reports mirrored networking as not enabled
- **THEN** the section shows the `[wsl2]` / `networkingMode=mirrored` snippet and the `wsl --shutdown` caveat

#### Scenario: No automatic .wslconfig edit

- **WHEN** the user interacts with any control in the WSL Integration section
- **THEN** `%USERPROFILE%\.wslconfig` is never written by the application

#### Scenario: Guidance hidden when already enabled

- **WHEN** detection reports mirrored networking as enabled
- **THEN** the snippet and caveat are not shown (or are collapsed as already satisfied)

### Requirement: Configure installs a managed crypt-env launcher

When the client configuration is applied to a distribution, the system SHALL also install a managed executable launcher named exactly `crypt-env` inside that distribution. The launcher SHALL run the Windows-installed `crypt-env.exe` belonging to the CryptEnv installation that performed the configuration, pass every argument through unchanged, and propagate its exit status, standard input, standard output and standard error. After a new shell is opened, typing `crypt-env` (with no `.exe` suffix) SHALL run that CLI. The launcher MUST NOT contain or cache any secret, token or certificate content.

#### Scenario: Command available without the .exe suffix

- **WHEN** the user runs Configure for a distribution and then opens a new interactive shell in it
- **THEN** `crypt-env --version` succeeds and prints the same version as the Windows GUI
- **AND** the exit status of `crypt-env` equals the exit status of the Windows CLI

#### Scenario: Arguments are forwarded verbatim

- **WHEN** the user runs `crypt-env search "my key"` through the launcher
- **THEN** the Windows CLI receives exactly the two arguments `search` and `my key`

#### Scenario: Reconfigure refreshes the launcher

- **WHEN** Configure is run again after CryptEnv was reinstalled in a different location
- **THEN** the launcher is rewritten to target the new location of the Windows CLI
- **AND** the rc files are left unchanged and no second backup is created

### Requirement: Windows CLI location is resolved per installation

The target of the launcher SHALL be derived at configuration time from the actual location of the running CryptEnv installation. It SHALL be translated into the distribution's own view of that path, honoring a non-default automount root when the distribution provides a path translator. The system MUST NOT hard-code any user name, drive letter, profile directory or install directory.

#### Scenario: Any user and install location

- **WHEN** CryptEnv is installed for Windows user `alice` under `D:\Apps\CryptEnv`
- **THEN** the launcher targets the distribution path corresponding to `D:\Apps\CryptEnv\crypt-env.exe`

#### Scenario: Custom automount root

- **WHEN** the distribution mounts Windows drives under `/win` instead of `/mnt`
- **THEN** the launcher targets `/win/<drive>/...`, not `/mnt/<drive>/...`

#### Scenario: Windows CLI not found

- **WHEN** no `crypt-env.exe` exists next to the running CryptEnv installation (for example, a development build)
- **THEN** the environment configuration is still applied
- **AND** no launcher is written
- **AND** the action report states that the launcher was skipped and why

### Requirement: Launcher never replaces foreign files and yields to native clients

The launcher SHALL be written only inside a directory owned by CryptEnv within the user's home directory. It SHALL be identifiable as managed by a fixed marker. If a file already exists at the launcher path without that marker, the system MUST NOT modify it and SHALL report the launcher as skipped. The launcher directory SHALL be appended to the end of `PATH` by the managed environment file, so that any `crypt-env` already found earlier on `PATH` (such as a native Linux build) takes precedence.

#### Scenario: Native Linux client takes precedence

- **WHEN** a native `crypt-env` exists in a directory already on `PATH` and Configure is run
- **THEN** running `crypt-env` in a new shell executes the native binary, not the launcher

#### Scenario: Foreign file at the launcher path

- **WHEN** a file without the managed marker exists at the launcher path
- **THEN** Configure leaves that file byte-for-byte unchanged
- **AND** the action report states that the launcher was skipped because a foreign file is present

#### Scenario: PATH entry is idempotent

- **WHEN** the managed environment file is sourced more than once in the same shell
- **THEN** the launcher directory appears in `PATH` exactly once

### Requirement: Launcher isolates WSL-side client configuration from the Windows CLI

The launcher SHALL remove `CRYPTENV_API_URL`, `CRYPTENV_CERT_PATH` and `CRYPTENV_TOKEN_PATH` from the environment it passes to the Windows CLI. The Windows CLI then uses its own Windows-side endpoint, certificate and token, even if the user forwards those variables through `WSLENV`.

#### Scenario: WSL variables are not forwarded

- **WHEN** `CRYPTENV_CERT_PATH` is set to a `/mnt/c/...` path in the WSL shell and listed in `WSLENV`
- **THEN** the Windows CLI started by the launcher does not see `CRYPTENV_CERT_PATH`

#### Scenario: Works without mirrored networking

- **WHEN** WSL uses the default NAT networking mode and the vault GUI is running and unlocked on Windows
- **THEN** commands run through the launcher reach the vault

### Requirement: Launcher failures are explicit

If the targeted Windows CLI is missing or cannot be executed at run time (for example, CryptEnv was uninstalled or WSL interop is disabled), the launcher SHALL exit with a non-zero status. It SHALL print a single-line message to standard error that names the expected path and tells the user to reinstall CryptEnv or re-run Configure. It MUST NOT fall back to any other executable.

#### Scenario: CryptEnv uninstalled

- **WHEN** the user runs `crypt-env` after uninstalling CryptEnv on Windows
- **THEN** the command exits non-zero with a message naming the missing path and suggesting reinstall or Configure

### Requirement: Remove deletes the launcher

The remove operation SHALL delete the launcher when it carries the managed marker. It SHALL remove the CryptEnv-owned launcher directory if that directory is then empty. It MUST NOT delete a file at the launcher path that lacks the managed marker. Removing when no launcher exists SHALL succeed as a no-op.

#### Scenario: Remove after configure

- **WHEN** the user chooses Remove for a configured distribution with a launcher
- **THEN** the launcher and its empty directory are deleted, and `crypt-env` is no longer resolved from them in a new shell
- **AND** the action report lists the deleted launcher

#### Scenario: Remove preserves foreign file

- **WHEN** a file without the managed marker exists at the launcher path and Remove is chosen
- **THEN** that file is left unchanged

### Requirement: Detection and reporting include launcher state

Detection SHALL report, per distribution, whether the managed launcher is installed, in addition to the existing configured flag. The settings panel SHALL distinguish a distribution that is configured but lacks the launcher and SHALL offer Reconfigure for it. The Configure and Remove reports SHALL state the launcher outcome: written (with its path), unchanged, skipped (with reason) or deleted.

#### Scenario: Configured before this change

- **WHEN** a distribution has the environment configuration but no launcher
- **THEN** the panel shows it as configured without the `crypt-env` command and offers Reconfigure

#### Scenario: Report lists the launcher

- **WHEN** Configure writes the launcher
- **THEN** the report shown in the panel includes the launcher path

### Requirement: Mirrored-networking guidance scoped to native Linux clients

When mirrored networking is not enabled, the guidance SHALL state that it is needed only by a native Linux `crypt-env` and that the managed launcher works without it. The existing rule that the application never writes `%USERPROFILE%\.wslconfig` SHALL remain in force.

#### Scenario: Guidance wording

- **WHEN** detection reports mirrored networking as not enabled
- **THEN** the banner explains that the Configure launcher works without mirrored networking and that the snippet applies only to a native Linux client
