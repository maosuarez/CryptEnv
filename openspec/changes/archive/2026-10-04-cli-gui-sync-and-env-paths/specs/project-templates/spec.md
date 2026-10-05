## MODIFIED Requirements

### Requirement: Atomic project scaffolding
Confirming the review step SHALL create, as a single all-or-nothing operation: the project (name, description, categories, template = comma-joined selected template ids in selection order, or `generic` if none), its `default` environment (plus empty `staging` and `production` environments when no custom initial environment was named), one project-scoped (non-global) `secret` vault item per variable (item name = key, value = edited value, or `CHANGE_ME` if empty), and the links from the default environment to those items under their keys. The operation MUST require an unlocked vault. The operation MUST reject: an invalid project name (same rules as normal project save), any key not matching `^[A-Z_][A-Z0-9_]*$`, duplicate keys, and more than 200 variables. If any step fails, the operation MUST leave no partial project, environment, or vault item behind.

#### Scenario: Successful scaffold
- **WHEN** the user confirms a project `shop` with Node.js + PostgreSQL selected and 9 variables
- **THEN** project `shop` exists with template `node,postgres`, a default environment containing 9 variables, each linked to a new encrypted project-scoped secret item
- **AND** empty `staging` and `production` environments exist beside it

#### Scenario: Empty value placeholder
- **WHEN** a variable is confirmed with an empty value
- **THEN** its secret item is stored with value `CHANGE_ME`

#### Scenario: Failure rolls back
- **WHEN** creating the 5th vault item fails
- **THEN** the project and all items created so far are removed and an error is shown

#### Scenario: Locked vault
- **WHEN** the scaffold is requested while the vault is locked
- **THEN** the request is rejected and nothing is created

#### Scenario: Invalid key rejected by backend
- **WHEN** the scaffold request contains key `my-key`
- **THEN** the request is rejected and nothing is created
