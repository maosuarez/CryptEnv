## ADDED Requirements

### Requirement: Preserving Project Templates on New Environment Creation
When adding a new environment to an existing project via "+ Add Environment", the application SHALL preserve the stack templates associated with the project. The new environment creation workflow MUST allow pre-filling or scaffolding the new environment with the variables defined by the project's templates, ensuring consistent variable definitions across environments.

#### Scenario: Adding an environment to a project with templates
- **WHEN** the user clicks "+ Add Environment" in a project that has configured templates (e.g., `nextjs,supabase`)
- **THEN** the new environment workflow provides the template variables derived from the project's templates
- **AND** the user can review and populate values for those template variables in the new environment.
