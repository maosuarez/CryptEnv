## Purpose

Describes what the platform installers deliver alongside the CryptEnv desktop GUI, so that the command-line tooling is available without a separate manual build.

## ADDED Requirements

### Requirement: Windows installer ships the command-line executables

The Windows installer SHALL install the `crypt-env` and `crypt-env-mcp` executables together with the GUI application, and SHALL make `crypt-env` invocable by name from a newly opened Windows shell. Installers for other platforms SHALL be unaffected by this requirement.

#### Scenario: CLI present after install

- **WHEN** a user installs CryptEnv on Windows and opens a new terminal
- **THEN** running `crypt-env --version` succeeds and prints the same version as the installed GUI

#### Scenario: MCP server present after install

- **WHEN** the Windows installation completes
- **THEN** `crypt-env-mcp` is installed next to `crypt-env` and can be launched as an MCP stdio server

#### Scenario: Uninstall removes the executables

- **WHEN** the user uninstalls CryptEnv on Windows
- **THEN** the `crypt-env` and `crypt-env-mcp` executables are removed and the `PATH` entry added at install time is reverted

#### Scenario: Other platforms unchanged

- **WHEN** the macOS or Linux bundle is produced
- **THEN** its contents are unchanged by this requirement

#### Scenario: Bundled CLI matches the GUI build

- **WHEN** a release is built
- **THEN** the bundled `crypt-env` / `crypt-env-mcp` are built from the same source revision as the GUI in that release
