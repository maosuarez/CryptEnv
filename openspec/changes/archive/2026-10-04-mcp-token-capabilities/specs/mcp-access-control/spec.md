## Purpose

Defines what an MCP-token caller may do through the local REST API, and the human-approval flow for sensitive operations. The goal is that no secret value or decryption material ever reaches an MCP caller.

## ADDED Requirements

### Requirement: Caller principal distinction

The REST API MUST classify every authenticated request as either a *session* principal (a password-derived CLI or GUI session token) or the *MCP* principal (the static MCP token). Token comparison MUST remain constant-time. The classification SHALL be available to every route handler.

#### Scenario: MCP token identified
- **WHEN** a request carries the configured MCP token
- **THEN** the handler sees the MCP principal, not a session principal

### Requirement: Operations denied to the MCP principal

The API MUST reject the following requests from the MCP principal with HTTP 403 and code `MCP_FORBIDDEN`, performing no side effect:
- revealing any item's secret value;
- confirming a LAN pairing fingerprint;
- changing `auto_lock_timeout`, rotating the MCP token, or changing biometric settings;
- `/fill` or `/environments/:id/example` with an `output_path` that does not resolve inside a registered project root;
- any write that would overwrite a pre-existing file crypt-env does not manage.

#### Scenario: MCP reveal attempt
- **WHEN** the MCP token calls `GET /items/42/reveal`
- **THEN** the response is 403 `MCP_FORBIDDEN` and no secret value appears in the body

#### Scenario: MCP disables auto-lock
- **WHEN** the MCP token sends `PUT /settings` with `auto_lock_timeout: 0`
- **THEN** the response is 403 and the setting is unchanged

#### Scenario: MCP output path outside projects
- **WHEN** the MCP token calls `/fill` with `output_path: "/home/u/.bashrc"`
- **THEN** the response is 403 and the file is untouched

#### Scenario: Session caller unaffected
- **WHEN** a session principal performs the same reveal
- **THEN** the existing behavior is unchanged

### Requirement: Human approval for sensitive MCP operations

Relay send, share-package export, and adding, updating or deleting MCP host server entries requested by the MCP principal MUST NOT execute immediately. The API SHALL create a pending approval containing a non-secret summary: operation, item names and count, destination, and requesting principal. It SHALL respond 202 with an approval id and status `pending`. The desktop app SHALL show the request to the user. The operation SHALL execute only after the user approves it in the desktop app.

A pending approval MUST expire after 120 seconds, MUST be discarded when the vault locks, and at most 8 SHALL be pending at once (further requests get 429). The MCP caller MAY poll the approval status. The final status SHALL be one of `approved`, `denied` or `expired`, plus non-secret result metadata.

#### Scenario: Approved relay send
- **WHEN** an MCP agent requests `relay_send` for 3 items and the user clicks Approve in the desktop app
- **THEN** the relay upload happens, and the desktop app shows the relay code and passphrase to the user
- **AND** polling the approval returns `approved` with the item count, but no code and no passphrase

#### Scenario: Denied or ignored
- **WHEN** the user clicks Deny, or does not answer within 120 seconds
- **THEN** nothing is uploaded or written, and polling returns `denied` or `expired`

#### Scenario: Lock discards pending approvals
- **WHEN** the vault locks while an approval is pending
- **THEN** the approval is discarded, and it cannot be approved after the next unlock

### Requirement: No decryption material to MCP callers

No response to the MCP principal SHALL contain a secret value, a relay passphrase, a relay code, a share-package passphrase, or the MCP token itself. This SHALL hold for success and error responses alike.

#### Scenario: Error path
- **WHEN** an approved export fails after the passphrase was generated
- **THEN** the MCP-visible error contains no passphrase
