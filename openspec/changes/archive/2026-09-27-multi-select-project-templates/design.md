## Context

See `proposal.md` → Why. Current state relevant to the design:

- `TEMPLATES` is a 6-entry constant in `src/components/ProjectManager.tsx`; `TemplateModal` returns one id, stored verbatim in `projects.template` (`TEXT NOT NULL DEFAULT 'generic'`). The listed `vars` are only used to render a "N vars" label — nothing is created.
- `project::save_project` auto-creates a `default` environment for new projects and silently drops category names that don't exist (`category_names_to_ids`).
- Every environment variable must point at a vault item. Project-scoped items are created by `vault::create_project_item(db, key, item, project_id)` (needs the in-memory `CryptoKey`, hence lives in `vault`). `environment_save` links vars by `itemId`.
- `db::delete_project` returns `ProjectDeleteImpact { items_deleted, … }` — it deletes items owned solely by that project, which makes it a usable compensating action.
- No backend validation of env var keys exists today (only frontend normalisation to `A-Z0-9_`).

## Goals / Non-Goals

**Goals:**
- Catalog is pure data + pure helpers, unit-testable with vitest, no React dependency.
- One IPC round-trip creates project + items + environment, with no partial state on failure.
- Picker stays usable with ~60+ templates (search, groups, keyboard).

**Non-Goals:**
- Sharing the catalog with the CLI/TUI (would require moving it to Rust; deferred).
- True SQL transactions across `db` helpers (they each use the pool directly; refactoring them to accept a transaction is out of scope).

## Decisions

### D1 — Catalog lives in the frontend (`src/data/projectTemplates.ts`)
Static typed array `PROJECT_TEMPLATES: TemplateDef[]` with `{ id, label, group, keywords, category, vars: { key, example, sensitive }[] }`, plus pure helpers `searchTemplates(query)` and `mergeTemplateVars(selectedIds)` returning `{ key, value, sensitive, source, alsoIn[] }[]` (first-wins dedup, selection order).
- *Alternative*: Rust catalog exposed via `project_list_templates`. Rejected for now: the only consumer is the GUI; a Rust catalog adds an IPC call and serde types without user value. If the CLI later needs templates, the data moves to a JSON file under `src-tauri/` consumed by both.
- The backend never trusts catalog ids — it receives the final `{key, value}` list, so the catalog cannot become an injection vector.

### D2 — New command `project_create_from_templates` in `vault/mod.rs`
```
project_create_from_templates(input: {
  project: ProjectInput,          // name, description, template (joined ids), categories
  vars: [{ key: String, value: String }]
}) -> Result<i64 /* project id */, String>
```
Pure fn `create_project_from_templates(db, key, input)`:
1. Validate up front (before any write): `validate_project_name`, `vars.len() <= 200`, each key `^[A-Z_][A-Z0-9_]*$`, keys unique. Errors mention key/index only.
2. `project::save_project` (creates project + `default` env + categories).
3. For each var: build `VaultItem::Secret { name: key, value, is_global: false, categories: [] }` (value `CHANGE_ME` if empty) → `create_project_item`.
4. `project::save_environment` on the default env with all `{key, item_id}`.
5. On error at 3–4: `db.delete_project(project_id)` (removes solely-owned items), then return the original error. If rollback also fails, return an error stating the project id so the user can delete it manually.
- Lives in `vault` because it needs the `CryptoKey` and orchestrates `project` + `db` (respects "vault orchestrates both").
- *Alternative A*: orchestrate from React (`project_save` → N × `vault_create_project_item` → `environment_save`). Rejected: N+2 IPC calls and no rollback — a mid-way failure leaves a half-built project with orphan-looking items.
- *Alternative B*: wrap in a real SQLite transaction. Rejected for this change: every `db` helper takes `&self.pool`; threading a `Transaction` through would be a large unrelated refactor. Compensating delete gives the same observable guarantee for a single-user local app.

### D3 — Categories
Frontend computes missing category names (vs `useVaultStore().cats`) and calls the existing `saveCats` (`vault_save_categories`) with new `{ id, name, color }` entries **before** calling the scaffold command. Colors come from a fixed palette cycled by index. Category creation is not rolled back on scaffold failure — harmless (empty tags) and keeps the backend command free of category concerns. Name match is case-sensitive, consistent with `category_names_to_ids`.

### D4 — `template` column format
Comma-joined ids in selection order (`node,postgres,openai`), `generic` when none. No migration: column is `TEXT`; legacy single ids are valid one-element lists. TS `ProjectTemplate` becomes `string`. The project card/form display splits on `,` and maps ids to labels when known, raw id otherwise.

### D5 — UI flow
`New project` → **Step 1 picker** (modal, wider: `max-w-2xl`, fixed-height scroll area): search input autofocused, groups as section headers, templates as toggle chips/cards with var count; footer shows `N selected`, `Clear`, `Cancel`, `Continue`. → **Step 2 project form** (existing form) extended with a **"Variables" review panel**: rows grouped by source template, editable key + value (masked when sensitive, eye toggle), "also in X" hint, remove button, "+ Add variable". Categories chip input prefilled. `Create` calls the store's new `createFromTemplates`. For existing projects the template field is read-only and the "change template" button is removed.
Keyboard: `/` or typing focuses search, `Enter` on a focused card toggles, `Esc` cancels. Styling uses existing tokens (`bg-surface`, `border-bd`, `text-accent`…), no inline styles.

### D6 — Catalog content
Keys and examples follow the upstream projects' documented conventions (sources below). Sensitive vars have empty example (→ `CHANGE_ME`) or an example URL containing `CHANGE_ME`. `.NET`-style `Section__Key` names are uppercased (configuration binding is case-insensitive).

| Group | id → label: variables (`*` = sensitive) |
|---|---|
| Runtimes & frameworks | `generic`-less. `node` Node.js: NODE_ENV=development, PORT=3000, LOG_LEVEL=info · `express` Express: PORT=3000, SESSION_SECRET*, CORS_ORIGIN=http://localhost:5173 · `nextjs` Next.js: NEXT_PUBLIC_APP_URL=http://localhost:3000, NEXT_TELEMETRY_DISABLED=1 · `nestjs` NestJS: PORT=3000, JWT_SECRET*, JWT_EXPIRES_IN=1h · `vite` Vite: VITE_API_URL=http://localhost:3000 · `python` Python: PYTHONUNBUFFERED=1, LOG_LEVEL=INFO · `django` Django: DJANGO_SECRET_KEY*, DJANGO_DEBUG=True, DJANGO_ALLOWED_HOSTS=localhost,127.0.0.1, DJANGO_SETTINGS_MODULE=config.settings · `flask` Flask: FLASK_APP=app.py, FLASK_DEBUG=1, SECRET_KEY* · `fastapi` FastAPI: SECRET_KEY*, BACKEND_CORS_ORIGINS=http://localhost:5173, UVICORN_HOST=127.0.0.1, UVICORN_PORT=8000 · `java` Java/JVM: JAVA_OPTS=-Xms256m -Xmx512m · `spring` Spring Boot: SPRING_PROFILES_ACTIVE=dev, SERVER_PORT=8080, SPRING_DATASOURCE_URL=jdbc:postgresql://localhost:5432/app, SPRING_DATASOURCE_USERNAME=app, SPRING_DATASOURCE_PASSWORD* · `quarkus` Quarkus: QUARKUS_PROFILE=dev, QUARKUS_HTTP_PORT=8080, QUARKUS_DATASOURCE_JDBC_URL=jdbc:postgresql://localhost:5432/app, QUARKUS_DATASOURCE_USERNAME=app, QUARKUS_DATASOURCE_PASSWORD* · `go` Go: APP_ENV=development, PORT=8080 · `rails` Ruby on Rails: RAILS_ENV=development, RAILS_MASTER_KEY*, DATABASE_URL=postgres://localhost:5432/app_development · `laravel` Laravel: APP_NAME=Laravel, APP_ENV=local, APP_KEY*, APP_DEBUG=true, APP_URL=http://localhost, DB_CONNECTION=mysql, DB_HOST=127.0.0.1, DB_PORT=3306, DB_DATABASE=laravel, DB_USERNAME=root, DB_PASSWORD* · `dotnet` ASP.NET Core: ASPNETCORE_ENVIRONMENT=Development, ASPNETCORE_URLS=http://localhost:5000, CONNECTIONSTRINGS__DEFAULTCONNECTION* |
| Databases | `postgres` PostgreSQL: POSTGRES_HOST=localhost, POSTGRES_PORT=5432, POSTGRES_USER=postgres, POSTGRES_PASSWORD*, POSTGRES_DB=app, DATABASE_URL*=postgresql://postgres:CHANGE_ME@localhost:5432/app · `mysql` MySQL: MYSQL_HOST=localhost, MYSQL_PORT=3306, MYSQL_USER=app, MYSQL_PASSWORD*, MYSQL_ROOT_PASSWORD*, MYSQL_DATABASE=app · `mariadb` MariaDB: MARIADB_USER=app, MARIADB_PASSWORD*, MARIADB_ROOT_PASSWORD*, MARIADB_DATABASE=app · `mongo` MongoDB: MONGODB_URI*=mongodb://root:CHANGE_ME@localhost:27017, MONGODB_DB=app, MONGO_INITDB_ROOT_USERNAME=root, MONGO_INITDB_ROOT_PASSWORD* · `sqlserver` SQL Server: MSSQL_SA_PASSWORD*, ACCEPT_EULA=Y, MSSQL_PID=Developer · `sqlite` SQLite: DATABASE_URL=sqlite:./data/app.db · `prisma` Prisma: DATABASE_URL*=postgresql://postgres:CHANGE_ME@localhost:5432/app, SHADOW_DATABASE_URL* · `supabase` Supabase: SUPABASE_URL=http://localhost:54321, SUPABASE_ANON_KEY*, SUPABASE_SERVICE_ROLE_KEY* · `firebase` Firebase: FIREBASE_PROJECT_ID, GOOGLE_APPLICATION_CREDENTIALS=./service-account.json · `elasticsearch` Elasticsearch: ELASTICSEARCH_URL=http://localhost:9200, ELASTIC_PASSWORD* · `neo4j` Neo4j: NEO4J_URI=bolt://localhost:7687, NEO4J_USERNAME=neo4j, NEO4J_PASSWORD* |
| Caches & queues | `redis` Redis: REDIS_URL=redis://localhost:6379, REDIS_PASSWORD* · `rabbitmq` RabbitMQ: RABBITMQ_DEFAULT_USER=app, RABBITMQ_DEFAULT_PASS*, AMQP_URL*=amqp://app:CHANGE_ME@localhost:5672 · `kafka` Kafka: KAFKA_BOOTSTRAP_SERVERS=localhost:9092, KAFKA_CLIENT_ID=app, KAFKA_GROUP_ID=app · `celery` Celery: CELERY_BROKER_URL=redis://localhost:6379/0, CELERY_RESULT_BACKEND=redis://localhost:6379/1 |
| AI APIs | `openai` OpenAI: OPENAI_API_KEY*, OPENAI_ORG_ID, OPENAI_BASE_URL=https://api.openai.com/v1 · `anthropic` Anthropic: ANTHROPIC_API_KEY*, ANTHROPIC_MODEL=claude-sonnet-5 · `gemini` Google Gemini: GEMINI_API_KEY* · `azure-openai` Azure OpenAI: AZURE_OPENAI_API_KEY*, AZURE_OPENAI_ENDPOINT=https://CHANGE_ME.openai.azure.com, OPENAI_API_VERSION=2024-10-21 · `mistral` Mistral: MISTRAL_API_KEY* · `groq` Groq: GROQ_API_KEY* · `openrouter` OpenRouter: OPENROUTER_API_KEY* · `huggingface` Hugging Face: HF_TOKEN* · `ollama` Ollama: OLLAMA_HOST=http://localhost:11434 · `langsmith` LangChain/LangSmith: LANGSMITH_TRACING=true, LANGSMITH_API_KEY*, LANGSMITH_PROJECT=app · `pinecone` Pinecone: PINECONE_API_KEY*, PINECONE_INDEX=app |
| Cloud & DevOps | `docker` Docker: COMPOSE_PROJECT_NAME=app, DOCKER_REGISTRY=docker.io, DOCKER_USERNAME, DOCKER_PASSWORD*, IMAGE_TAG=latest · `aws` AWS: AWS_ACCESS_KEY_ID*, AWS_SECRET_ACCESS_KEY*, AWS_REGION=us-east-1 · `gcp` Google Cloud: GOOGLE_CLOUD_PROJECT, GOOGLE_APPLICATION_CREDENTIALS=./service-account.json · `azure` Azure: AZURE_TENANT_ID, AZURE_CLIENT_ID, AZURE_CLIENT_SECRET*, AZURE_SUBSCRIPTION_ID · `minio` MinIO/S3: MINIO_ROOT_USER=minioadmin, MINIO_ROOT_PASSWORD*, S3_ENDPOINT=http://localhost:9000 · `cloudflare` Cloudflare: CLOUDFLARE_API_TOKEN*, CLOUDFLARE_ACCOUNT_ID · `vercel` Vercel: VERCEL_TOKEN* · `github` GitHub: GITHUB_TOKEN* |
| Auth | `jwt` JWT: JWT_SECRET*, JWT_EXPIRES_IN=15m · `authjs` Auth.js: AUTH_SECRET*, AUTH_URL=http://localhost:3000 · `auth0` Auth0: AUTH0_DOMAIN, AUTH0_CLIENT_ID, AUTH0_CLIENT_SECRET*, AUTH0_SECRET*, APP_BASE_URL=http://localhost:3000 · `clerk` Clerk: NEXT_PUBLIC_CLERK_PUBLISHABLE_KEY, CLERK_SECRET_KEY* · `google-oauth` Google OAuth: GOOGLE_CLIENT_ID, GOOGLE_CLIENT_SECRET* |
| Payments & messaging | `stripe` Stripe: STRIPE_SECRET_KEY*, STRIPE_PUBLISHABLE_KEY, STRIPE_WEBHOOK_SECRET* · `smtp` SMTP: SMTP_HOST=localhost, SMTP_PORT=587, SMTP_USER, SMTP_PASSWORD*, MAIL_FROM=no-reply@example.com · `sendgrid` SendGrid: SENDGRID_API_KEY* · `resend` Resend: RESEND_API_KEY* · `twilio` Twilio: TWILIO_ACCOUNT_SID, TWILIO_AUTH_TOKEN* · `slack` Slack: SLACK_BOT_TOKEN*, SLACK_SIGNING_SECRET* |
| Observability | `sentry` Sentry: SENTRY_DSN, SENTRY_ENVIRONMENT=development · `otel` OpenTelemetry: OTEL_SERVICE_NAME=app, OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318 · `datadog` Datadog: DD_API_KEY*, DD_SITE=datadoghq.com, DD_ENV=dev |

≈ 64 templates. Vars with no example and not sensitive (e.g. `AUTH0_DOMAIN`) also fall back to `CHANGE_ME`.

**Sources** (upstream docs/`.env.example` conventions): official Docker Hub images for `postgres`, `mysql`, `mariadb`, `mongo`, `mcr.microsoft.com/mssql/server`, `rabbitmq`, `elasticsearch`, `neo4j`, `minio/minio`; Prisma docs (`DATABASE_URL`, `shadowDatabaseUrl`); Spring Boot relaxed binding & Quarkus config env mapping; Laravel `.env.example`; cookiecutter-django; tiangolo/full-stack-fastapi-template; Uvicorn `UVICORN_*` settings; Flask CLI env; Rails credentials (`RAILS_MASTER_KEY`); ASP.NET Core configuration env vars; OpenAI, Anthropic, Google GenAI, Azure OpenAI, Mistral, Groq, Hugging Face Hub, Ollama, LangSmith, Pinecone SDK env conventions; AWS CLI/SDK, Google ADC, Azure `EnvironmentCredential`; Auth.js, nextjs-auth0 v4, Clerk, Stripe, Twilio, Slack Bolt, Sentry SDK, OpenTelemetry SDK env spec, Datadog agent docs. Implementation task includes a spot-check of each against current docs.

## Security & Threat Model

- **Data in transit**: example/edited values cross Tauri IPC once, same trust boundary as `vault_create_project_item` today. No new network, REST, or MCP surface.
- **At rest**: each value becomes a normal encrypted secret item (AES-256-GCM under the session key). Nothing is written to `projects.template` except ids.
- **Leakage**: validation and rollback errors reference key names / indices / counts only. No `println!`/`log` of the input struct; the input type does not derive `Debug` (or the value field is redacted) to prevent accidental formatting.
- **Locked vault**: command reads `s.key.as_ref().ok_or("vault is locked")` before any write.
- **Abuse bounds**: ≤ 200 vars, key regex enforced server-side (the frontend normalisation is UX only, mirroring the existing name-validation stance).
- **Catalog**: static bundled data; ids are never interpreted by the backend.
- **Placeholder**: `CHANGE_ME` is intentionally not a secret; injecting an un-edited environment writes `CHANGE_ME`, which fails loudly at the consuming service rather than silently using a weak default.

## Risks / Trade-offs

- [Compensating rollback isn't a DB transaction; crash mid-operation could leave a partial project] → Local single-user app, window is milliseconds; partial project is visible and deletable with existing delete flow, which cleans owned items.
- [Catalog drifts from upstream conventions over time] → Data is isolated in one file with a source per template; easy to update.
- [Large catalog makes the picker heavy on small windows] → Fixed-height scroll area, grouped headers, search autofocus.
- [Category creation happens before scaffold and isn't rolled back] → Only adds empty tag names; acceptable and documented.
- [Removing "change template" on existing projects] → It never had any effect; showing it read-only removes a misleading control.

## Migration Plan

No DB migration. Legacy template strings render as-is. Rollback = revert the commit; projects created with comma-joined templates still load in the old UI (displayed as a raw string).
