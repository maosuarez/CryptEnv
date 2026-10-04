# relay-sharing Specification

## Purpose
Defines the access, single-use and expiry guarantees of the internet relay used for ephemeral secret sharing. These guarantees are enforced by the relay database itself, not by client cooperation.

## Requirements

### Requirement: Server-enforced single use and expiry

A relay package MUST be retrievable at most once. The retrieval SHALL atomically delete the package and return its payload. A package MUST NOT be returned after its expiry, which SHALL be 24 hours after upload. Expired packages SHALL be deleted no later than the next relay operation that any client performs.

#### Scenario: Second retrieval
- **WHEN** a receiver claims code `ABC-123` successfully and anyone claims it again
- **THEN** the second claim returns "not found or already used"

#### Scenario: Expired package
- **WHEN** a package is claimed 25 hours after upload
- **THEN** the claim returns "not found or already used", and the row no longer exists

### Requirement: No direct table access for relay clients

Relay clients using the anonymous key MUST NOT be able to list, read, update or delete relay rows directly. The only permitted operations SHALL be: uploading a new package (payload at most 1 MiB), and claiming a package by its code. The relay SHALL store only a domain-separated hash of the code, never the code in plaintext.

#### Scenario: Enumeration attempt
- **WHEN** a client with the anon key requests `GET /rest/v1/relay_packages?select=*`
- **THEN** the request is denied or returns no rows

#### Scenario: Tampering attempt
- **WHEN** a client with the anon key tries to `PATCH` or `DELETE` a relay row directly
- **THEN** the request is denied and the row is unchanged

### Requirement: Schema version detection

The app SHALL detect a relay backend that does not provide the v2 operations. When it does, the app SHALL fail the send or receive with a message telling the user to apply the relay SQL v2, and SHALL show that SQL. The app MUST NOT fall back to the v1 table access.

#### Scenario: Old schema
- **WHEN** the configured Supabase project still has the v1 schema and the user sends a relay share
- **THEN** the send fails with an "apply relay SQL v2" message, and no data is uploaded

### Requirement: Relay operation outcomes are checked

Every relay HTTP call SHALL have a 10-second connect timeout and a 30-second total timeout. A non-success status SHALL surface as an error. No relay error message SHALL contain the passphrase or the payload.

#### Scenario: Relay down
- **WHEN** the relay host does not answer
- **THEN** the operation fails within 30 seconds with a network error

### Requirement: Bounded key derivation

Relay key derivation MUST NOT run on async runtime worker threads. At most 2 derivations SHALL run concurrently in the process; additional requests SHALL wait.

#### Scenario: Burst of receives
- **WHEN** 10 relay receives are requested at once
- **THEN** at most 2 derivations run at a time, and other GUI and REST requests keep responding
