## Purpose

The Windows-only capability by which the CryptEnv GUI detects a WSL environment, reports its readiness for the "vault on Windows, CLI in WSL" workflow, and configures or removes the `crypt-env` client inside a chosen distribution without destroying existing shell content.

## ADDED Requirements

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
