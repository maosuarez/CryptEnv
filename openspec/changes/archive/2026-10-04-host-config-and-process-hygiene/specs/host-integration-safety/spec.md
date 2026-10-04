## Purpose

Defines how crypt-env modifies configuration files owned by other applications (MCP host configs such as `.mcp.json` or `claude_desktop_config.json`). Existing user content must never be lost, and credentials written there must be protected.

## ADDED Requirements

### Requirement: Existing configuration is never discarded

When adding or updating crypt-env's entry in an existing third-party configuration file, crypt-env SHALL change only its own entry. If the existing file can't be parsed strictly, crypt-env MUST NOT modify it, and SHALL fail with an error that includes the entry the user can add manually. The entry SHALL NOT include the token value in the error; a placeholder is shown instead.

#### Scenario: File with comments
- **WHEN** `.mcp.json` contains a `// comment` and two other servers, and the user generates the crypt-env MCP config
- **THEN** the file is unchanged, and the user sees an error with the snippet to add manually

#### Scenario: Valid file with other servers
- **WHEN** `.mcp.json` is valid JSON with two other servers
- **THEN** after generation it contains those two servers unchanged, plus the `cryptenv` entry

### Requirement: Atomic, private config writes

Writes to third-party configuration files SHALL be atomic: after any failure, the file has either its previous or its new content. Before the first modification of an existing file, a backup copy SHALL be kept next to it. A file created by crypt-env that contains a token SHALL be created with owner-only permissions on Unix.

#### Scenario: Interrupted write
- **WHEN** writing the updated config fails partway
- **THEN** the original file is intact
