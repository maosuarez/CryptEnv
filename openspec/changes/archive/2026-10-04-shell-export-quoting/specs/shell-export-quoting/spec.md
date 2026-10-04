## Purpose

Guarantees that any shell code crypt-env emits for a secret (CLI `inject`, GUI copy-as-shell) assigns exactly the stored value, and cannot execute any part of the key or value, whatever characters the secret contains.

## ADDED Requirements

### Requirement: Exact, non-executable assignments

For every supported shell (PowerShell, bash, zsh, sh), the emitted assignment, when evaluated by that shell, MUST set the variable to exactly the stored value, and MUST NOT execute any command contained in the value. For PowerShell, every character PowerShell treats as a single quote (U+0027, U+2018, U+2019, U+201A, U+201B) SHALL be escaped.

#### Scenario: PowerShell smart-quote breakout
- **WHEN** a secret's value is `’; Write-Output PWNED ;’` and the user runs `crypt-env inject KEY | Invoke-Expression` in PowerShell
- **THEN** `$env:KEY` equals the value exactly, and `PWNED` is not printed

#### Scenario: Command substitution in bash
- **WHEN** a value is `x$(touch /tmp/pwned)` and the user runs `eval "$(crypt-env inject KEY)"` in bash
- **THEN** `KEY` equals the value exactly, and `/tmp/pwned` is not created

#### Scenario: Newline in value
- **WHEN** a value contains a newline and the target shell is bash or zsh
- **THEN** the variable holds the value with the newline preserved

### Requirement: Valid variable names only

A shell assignment SHALL be emitted only for keys that match `^[A-Za-z_][A-Za-z0-9_]*$`. Otherwise, the operation SHALL fail with an error naming the key, and SHALL emit no shell code.

#### Scenario: Malicious key name
- **WHEN** an item's key is `A;rm -rf ~;B`
- **THEN** `inject` fails with an invalid-key error and prints nothing to stdout

### Requirement: Single formatter across interfaces

The GUI "copy as" actions SHALL produce the same text as the CLI for the same shell, key and value.

#### Scenario: GUI copy as bash
- **WHEN** the user copies a secret whose value contains a space and `$()` as bash
- **THEN** the clipboard text is a single-quoted `export` assignment identical to the CLI's bash output
