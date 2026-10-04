# rest-api-server Specification

## Purpose
Defines startup and error-reporting guarantees of the local REST server. Corrupt or mismatched TLS material heals itself, parsing never crashes the process, an unavailable server is visible to the user, and responses never expose internal details.

## Requirements

### Requirement: Self-healing TLS material

At startup, the server SHALL verify that the TLS certificate and private key both load, that the key matches the certificate, and that the certificate is not within 30 days of expiry. If any check fails, the server SHALL generate a new pair once and start with it. A certificate/key pair SHALL become active only as a whole; the server MUST NOT be able to load a certificate from one generation with a key from another.

#### Scenario: Crash between writes
- **WHEN** the previous run crashed while generating a new certificate
- **THEN** on the next launch the server starts successfully with a matching certificate and key

#### Scenario: Mismatched pair on disk
- **WHEN** the certificate and key on disk don't match
- **THEN** the server regenerates them and starts

### Requirement: Private key never exposed during creation

The private key file SHALL be created with owner-only permissions (0600 on Unix) at the moment it is created, and it SHALL never exist with broader permissions.

#### Scenario: Key generation
- **WHEN** a new key is generated on Linux with umask 022
- **THEN** the key file never exists with group or other permissions

### Requirement: Certificate parsing never crashes

Reading certificate metadata SHALL NOT panic or overflow for any input bytes. Malformed input SHALL be treated as an invalid certificate.

#### Scenario: Tampered certificate
- **WHEN** `cert.pem` contains a DER length field of `0xFFFFFFFFFFFFFFFF`, or non-ASCII time bytes
- **THEN** the server treats the certificate as invalid, regenerates it, and starts

### Requirement: Server status visible to the user

If the REST server fails to start, the desktop app SHALL show that the local API is unavailable and why, and SHALL offer to regenerate the TLS material and restart the server.

#### Scenario: Port in use
- **WHEN** port 47821 is already in use by another program
- **THEN** Settings shows "REST API unavailable: port in use" instead of failing silently

### Requirement: Generic internal error responses

Responses for internal errors SHALL contain a generic message, a stable error code, and a correlation id. They MUST NOT contain database error text, SQL, file system paths, or secret values. The detailed error SHALL be written only to the local log, together with the correlation id and without secret values.

#### Scenario: Database failure on delete
- **WHEN** deleting an item fails with a database error
- **THEN** the response is 500 with code `INTERNAL_ERROR` and a correlation id, and contains no SQL or schema text
