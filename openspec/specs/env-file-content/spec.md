# env-file-content Specification

## Purpose
Defines what crypt-env writes into `.env` files and how the write is performed. A written file always parses back to exactly the configured keys and values, a failed write never leaves partial secrets behind, and the writer can't be made to hang or exhaust memory.

## Requirements

### Requirement: Faithful dotenv serialization

Each variable SHALL be written as one logical `KEY=VALUE` line that common dotenv parsers read back as exactly `KEY` and the original value:
- a value consisting only of `[A-Za-z0-9_./:@+,=-]` SHALL be written unquoted;
- a value containing neither a newline nor `'` SHALL be written in single quotes;
- any other value SHALL be written in double quotes, with `\`, `"`, `$`, newline and carriage return escaped.

A value MUST NOT be able to introduce an additional variable or comment line. A key that does not match `^[A-Za-z_][A-Za-z0-9_.]*$` SHALL NOT be written, and SHALL be reported by key name.

#### Scenario: Multi-line PEM value
- **WHEN** an environment variable holds a PEM private key
- **THEN** the `.env` file contains one double-quoted line with `\n` escapes, and parsing it yields the exact PEM

#### Scenario: Injection attempt in a value
- **WHEN** a value is `x\nADMIN=1`
- **THEN** parsing the written file yields only the configured key, with value `x\nADMIN=1`, and no `ADMIN` key

### Requirement: Only regular files are read or replaced

Before reading or replacing a target, crypt-env MUST verify that the target is absent or a regular file. Devices, FIFOs, sockets and directories SHALL be refused with a `NOT_REGULAR_FILE` error, without being opened for reading. Reading an existing target to classify it SHALL read at most 1 MiB; a larger file SHALL be treated as not managed by crypt-env.

#### Scenario: Device as output path
- **WHEN** a session caller calls `/fill` with `output_path: "/dev/zero"`
- **THEN** the request fails with `NOT_REGULAR_FILE` promptly, and memory use does not grow

#### Scenario: FIFO target
- **WHEN** a configured path is a named pipe
- **THEN** injection for that path fails with `NOT_REGULAR_FILE` without blocking

### Requirement: Atomic replacement

Writing a `.env` file MUST be atomic: after any failure, the target SHALL contain either its complete previous content or the complete new content, and no temporary file SHALL remain. A newly written file SHALL have owner-only permissions from creation.

#### Scenario: Disk full mid-write
- **WHEN** the disk fills while writing a managed `.env`
- **THEN** the previous `.env` is intact, and no partial or temporary file with secrets remains

### Requirement: Minimal decryption

Injecting an environment MUST decrypt only the items referenced by that environment's variables. A referenced item that can't be decrypted SHALL be reported by key name. The other keys SHALL still be written, and the failure MUST NOT affect injections of other environments.

#### Scenario: Unrelated corrupt item
- **WHEN** an item not referenced by environment `dev` is corrupt, and the user injects `dev`
- **THEN** the injection succeeds
