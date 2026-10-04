## MODIFIED Requirements

### Requirement: WSL environment detection

The GUI SHALL provide a detection operation that reports: whether WSL is available; for each distribution its name, its run state, and, for running distributions, its default Linux user and whether the cryptenv client configuration is already installed in that distribution's shell startup; and whether `networkingMode=mirrored` is present in `%USERPROFILE%\.wslconfig`. Distribution names and states SHALL be obtained from the system WSL tooling and its output decoded correctly (UTF-16LE). Detection MUST NOT start a stopped distribution. A stopped distribution SHALL be reported as `stopped`, and the GUI SHALL offer an explicit per-distribution action to detect it, which the UI states will start the distribution. Docker Desktop's internal distributions SHALL be excluded. Every WSL tooling process that detection or configuration starts MUST be terminated and reaped when its operation times out or is abandoned. At most one WSL operation SHALL run at a time.

#### Scenario: Enumerate distributions

- **WHEN** the detection operation runs on Windows with two installed, running distributions
- **THEN** it returns both distribution names, each with its resolved default user and an installed/not-installed flag

#### Scenario: Stopped distribution not started

- **WHEN** the detection operation runs and distribution `Debian` is stopped
- **THEN** `Debian` is reported as `stopped`, and it is still stopped after detection

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

#### Scenario: Timed-out tooling is reaped

- **WHEN** a `wsl.exe` probe does not finish within its timeout
- **THEN** the process is killed, no `wsl.exe` started by crypt-env remains running, and the GUI shows a timeout state

#### Scenario: Concurrent operation rejected

- **WHEN** the user clicks Configure while a detection is still running
- **THEN** the second operation is rejected with "operation in progress"
