// Static, bundled catalog of project templates (see
// openspec/specs/project-templates). Keys and example values follow the
// conventions documented by each upstream project (official Docker images,
// SDK env conventions, framework `.env.example` files). Sensitive variables
// never carry a real credential: they are empty (stored as `CHANGE_ME`) or
// embed the `CHANGE_ME` placeholder.

export type TemplateGroup =
  | 'runtime' | 'database' | 'cache' | 'ai' | 'cloud' | 'auth' | 'payments' | 'observability';

export const TEMPLATE_GROUPS: TemplateGroup[] = [
  'runtime', 'database', 'cache', 'ai', 'cloud', 'auth', 'payments', 'observability',
];

export interface TemplateVar {
  key:       string;
  example:   string;
  sensitive: boolean;
}

export interface TemplateDef {
  id:       string;
  label:    string;
  group:    TemplateGroup;
  keywords: string[];
  /** Project category name prefilled when this template is selected. */
  category: string;
  vars:     TemplateVar[];
}

/** Stored by the backend in place of an empty value. */
export const TEMPLATE_PLACEHOLDER = 'CHANGE_ME';

const p = (key: string, example = ''): TemplateVar => ({ key, example, sensitive: false });
const s = (key: string, example = ''): TemplateVar => ({ key, example, sensitive: true });

const tpl = (
  id: string, label: string, group: TemplateGroup, keywords: string[], vars: TemplateVar[],
): TemplateDef => ({ id, label, group, keywords, category: label, vars });

export const PROJECT_TEMPLATES: TemplateDef[] = [
  // ── Runtimes & frameworks ──
  tpl('node', 'Node.js', 'runtime', ['javascript', 'js', 'npm'], [
    p('NODE_ENV', 'development'), p('PORT', '3000'), p('LOG_LEVEL', 'info'),
  ]),
  tpl('express', 'Express', 'runtime', ['node', 'javascript', 'api'], [
    p('PORT', '3000'), s('SESSION_SECRET'), p('CORS_ORIGIN', 'http://localhost:5173'),
  ]),
  tpl('nextjs', 'Next.js', 'runtime', ['react', 'node', 'vercel'], [
    p('NEXT_PUBLIC_APP_URL', 'http://localhost:3000'), p('NEXT_TELEMETRY_DISABLED', '1'),
  ]),
  tpl('nestjs', 'NestJS', 'runtime', ['node', 'typescript', 'api'], [
    p('PORT', '3000'), s('JWT_SECRET'), p('JWT_EXPIRES_IN', '1h'),
  ]),
  tpl('vite', 'Vite', 'runtime', ['react', 'vue', 'svelte', 'frontend'], [
    p('VITE_API_URL', 'http://localhost:3000'),
  ]),
  tpl('python', 'Python', 'runtime', ['py', 'pip'], [
    p('PYTHONUNBUFFERED', '1'), p('LOG_LEVEL', 'INFO'),
  ]),
  tpl('django', 'Django', 'runtime', ['python', 'py'], [
    s('DJANGO_SECRET_KEY'), p('DJANGO_DEBUG', 'True'),
    p('DJANGO_ALLOWED_HOSTS', 'localhost,127.0.0.1'), p('DJANGO_SETTINGS_MODULE', 'config.settings'),
  ]),
  tpl('flask', 'Flask', 'runtime', ['python', 'py'], [
    p('FLASK_APP', 'app.py'), p('FLASK_DEBUG', '1'), s('SECRET_KEY'),
  ]),
  tpl('fastapi', 'FastAPI', 'runtime', ['python', 'py', 'uvicorn', 'api'], [
    s('SECRET_KEY'), p('BACKEND_CORS_ORIGINS', 'http://localhost:5173'),
    p('UVICORN_HOST', '127.0.0.1'), p('UVICORN_PORT', '8000'),
  ]),
  tpl('java', 'Java', 'runtime', ['jvm', 'maven', 'gradle'], [
    p('JAVA_OPTS', '-Xms256m -Xmx512m'),
  ]),
  tpl('spring', 'Spring Boot', 'runtime', ['java', 'jvm', 'jdbc'], [
    p('SPRING_PROFILES_ACTIVE', 'dev'), p('SERVER_PORT', '8080'),
    p('SPRING_DATASOURCE_URL', 'jdbc:postgresql://localhost:5432/app'),
    p('SPRING_DATASOURCE_USERNAME', 'app'), s('SPRING_DATASOURCE_PASSWORD'),
  ]),
  tpl('quarkus', 'Quarkus', 'runtime', ['java', 'jvm', 'jdbc'], [
    p('QUARKUS_PROFILE', 'dev'), p('QUARKUS_HTTP_PORT', '8080'),
    p('QUARKUS_DATASOURCE_JDBC_URL', 'jdbc:postgresql://localhost:5432/app'),
    p('QUARKUS_DATASOURCE_USERNAME', 'app'), s('QUARKUS_DATASOURCE_PASSWORD'),
  ]),
  tpl('go', 'Go', 'runtime', ['golang'], [
    p('APP_ENV', 'development'), p('PORT', '8080'),
  ]),
  tpl('rails', 'Ruby on Rails', 'runtime', ['ruby'], [
    p('RAILS_ENV', 'development'), s('RAILS_MASTER_KEY'),
    p('DATABASE_URL', 'postgres://localhost:5432/app_development'),
  ]),
  tpl('laravel', 'Laravel', 'runtime', ['php', 'mysql'], [
    p('APP_NAME', 'Laravel'), p('APP_ENV', 'local'), s('APP_KEY'), p('APP_DEBUG', 'true'),
    p('APP_URL', 'http://localhost'), p('DB_CONNECTION', 'mysql'), p('DB_HOST', '127.0.0.1'),
    p('DB_PORT', '3306'), p('DB_DATABASE', 'laravel'), p('DB_USERNAME', 'root'), s('DB_PASSWORD'),
  ]),
  tpl('dotnet', 'ASP.NET Core', 'runtime', ['dotnet', 'csharp', 'c#', '.net'], [
    p('ASPNETCORE_ENVIRONMENT', 'Development'), p('ASPNETCORE_URLS', 'http://localhost:5000'),
    s('CONNECTIONSTRINGS__DEFAULTCONNECTION'),
  ]),

  // ── Databases ──
  tpl('postgres', 'PostgreSQL', 'database', ['postgres', 'sql', 'psql'], [
    p('POSTGRES_HOST', 'localhost'), p('POSTGRES_PORT', '5432'), p('POSTGRES_USER', 'postgres'),
    s('POSTGRES_PASSWORD'), p('POSTGRES_DB', 'app'),
    s('DATABASE_URL', 'postgresql://postgres:CHANGE_ME@localhost:5432/app'),
  ]),
  tpl('mysql', 'MySQL', 'database', ['sql', 'connector'], [
    p('MYSQL_HOST', 'localhost'), p('MYSQL_PORT', '3306'), p('MYSQL_USER', 'app'),
    s('MYSQL_PASSWORD'), s('MYSQL_ROOT_PASSWORD'), p('MYSQL_DATABASE', 'app'),
  ]),
  tpl('mariadb', 'MariaDB', 'database', ['sql', 'mysql'], [
    p('MARIADB_USER', 'app'), s('MARIADB_PASSWORD'), s('MARIADB_ROOT_PASSWORD'),
    p('MARIADB_DATABASE', 'app'),
  ]),
  tpl('mongo', 'MongoDB', 'database', ['mongo', 'nosql', 'mongoose'], [
    s('MONGODB_URI', 'mongodb://root:CHANGE_ME@localhost:27017'), p('MONGODB_DB', 'app'),
    p('MONGO_INITDB_ROOT_USERNAME', 'root'), s('MONGO_INITDB_ROOT_PASSWORD'),
  ]),
  tpl('sqlserver', 'SQL Server', 'database', ['mssql', 'sql', 'microsoft'], [
    s('MSSQL_SA_PASSWORD'), p('ACCEPT_EULA', 'Y'), p('MSSQL_PID', 'Developer'),
  ]),
  tpl('sqlite', 'SQLite', 'database', ['sql', 'file'], [
    p('DATABASE_URL', 'sqlite:./data/app.db'),
  ]),
  tpl('prisma', 'Prisma', 'database', ['orm', 'node', 'sql'], [
    s('DATABASE_URL', 'postgresql://postgres:CHANGE_ME@localhost:5432/app'), s('SHADOW_DATABASE_URL'),
  ]),
  tpl('supabase', 'Supabase', 'database', ['postgres', 'baas'], [
    p('SUPABASE_URL', 'http://localhost:54321'), s('SUPABASE_ANON_KEY'), s('SUPABASE_SERVICE_ROLE_KEY'),
  ]),
  tpl('firebase', 'Firebase', 'database', ['google', 'firestore', 'baas'], [
    p('FIREBASE_PROJECT_ID'), p('GOOGLE_APPLICATION_CREDENTIALS', './service-account.json'),
  ]),
  tpl('elasticsearch', 'Elasticsearch', 'database', ['search', 'elastic'], [
    p('ELASTICSEARCH_URL', 'http://localhost:9200'), s('ELASTIC_PASSWORD'),
  ]),
  tpl('neo4j', 'Neo4j', 'database', ['graph'], [
    p('NEO4J_URI', 'bolt://localhost:7687'), p('NEO4J_USERNAME', 'neo4j'), s('NEO4J_PASSWORD'),
  ]),

  // ── Caches & queues ──
  tpl('redis', 'Redis', 'cache', ['cache', 'valkey'], [
    p('REDIS_URL', 'redis://localhost:6379'), s('REDIS_PASSWORD'),
  ]),
  tpl('rabbitmq', 'RabbitMQ', 'cache', ['amqp', 'queue'], [
    p('RABBITMQ_DEFAULT_USER', 'app'), s('RABBITMQ_DEFAULT_PASS'),
    s('AMQP_URL', 'amqp://app:CHANGE_ME@localhost:5672'),
  ]),
  tpl('kafka', 'Kafka', 'cache', ['queue', 'stream'], [
    p('KAFKA_BOOTSTRAP_SERVERS', 'localhost:9092'), p('KAFKA_CLIENT_ID', 'app'), p('KAFKA_GROUP_ID', 'app'),
  ]),
  tpl('celery', 'Celery', 'cache', ['python', 'queue', 'worker'], [
    p('CELERY_BROKER_URL', 'redis://localhost:6379/0'), p('CELERY_RESULT_BACKEND', 'redis://localhost:6379/1'),
  ]),

  // ── AI APIs ──
  tpl('openai', 'OpenAI', 'ai', ['gpt', 'llm', 'chatgpt'], [
    s('OPENAI_API_KEY'), p('OPENAI_ORG_ID'), p('OPENAI_BASE_URL', 'https://api.openai.com/v1'),
  ]),
  tpl('anthropic', 'Anthropic', 'ai', ['claude', 'llm'], [
    s('ANTHROPIC_API_KEY'), p('ANTHROPIC_MODEL', 'claude-sonnet-5'),
  ]),
  tpl('gemini', 'Google Gemini', 'ai', ['google', 'llm', 'genai'], [
    s('GEMINI_API_KEY'),
  ]),
  tpl('azure-openai', 'Azure OpenAI', 'ai', ['azure', 'gpt', 'llm'], [
    s('AZURE_OPENAI_API_KEY'), p('AZURE_OPENAI_ENDPOINT', 'https://CHANGE_ME.openai.azure.com'),
    p('OPENAI_API_VERSION', '2024-10-21'),
  ]),
  tpl('mistral', 'Mistral', 'ai', ['llm'], [s('MISTRAL_API_KEY')]),
  tpl('groq', 'Groq', 'ai', ['llm'], [s('GROQ_API_KEY')]),
  tpl('openrouter', 'OpenRouter', 'ai', ['llm'], [s('OPENROUTER_API_KEY')]),
  tpl('huggingface', 'Hugging Face', 'ai', ['hf', 'transformers', 'llm'], [s('HF_TOKEN')]),
  tpl('ollama', 'Ollama', 'ai', ['local', 'llm'], [p('OLLAMA_HOST', 'http://localhost:11434')]),
  tpl('langsmith', 'LangChain / LangSmith', 'ai', ['langchain', 'llm', 'tracing'], [
    p('LANGSMITH_TRACING', 'true'), s('LANGSMITH_API_KEY'), p('LANGSMITH_PROJECT', 'app'),
  ]),
  tpl('pinecone', 'Pinecone', 'ai', ['vector', 'embeddings'], [
    s('PINECONE_API_KEY'), p('PINECONE_INDEX', 'app'),
  ]),

  // ── Cloud & DevOps ──
  tpl('docker', 'Docker', 'cloud', ['compose', 'container', 'registry'], [
    p('COMPOSE_PROJECT_NAME', 'app'), p('DOCKER_REGISTRY', 'docker.io'), p('DOCKER_USERNAME'),
    s('DOCKER_PASSWORD'), p('IMAGE_TAG', 'latest'),
  ]),
  tpl('aws', 'AWS', 'cloud', ['amazon', 's3'], [
    s('AWS_ACCESS_KEY_ID'), s('AWS_SECRET_ACCESS_KEY'), p('AWS_REGION', 'us-east-1'),
  ]),
  tpl('gcp', 'Google Cloud', 'cloud', ['gcp', 'google'], [
    p('GOOGLE_CLOUD_PROJECT'), p('GOOGLE_APPLICATION_CREDENTIALS', './service-account.json'),
  ]),
  tpl('azure', 'Azure', 'cloud', ['microsoft'], [
    p('AZURE_TENANT_ID'), p('AZURE_CLIENT_ID'), s('AZURE_CLIENT_SECRET'), p('AZURE_SUBSCRIPTION_ID'),
  ]),
  tpl('minio', 'MinIO / S3', 'cloud', ['s3', 'storage', 'object'], [
    p('MINIO_ROOT_USER', 'minioadmin'), s('MINIO_ROOT_PASSWORD'), p('S3_ENDPOINT', 'http://localhost:9000'),
  ]),
  tpl('cloudflare', 'Cloudflare', 'cloud', ['workers', 'cdn'], [
    s('CLOUDFLARE_API_TOKEN'), p('CLOUDFLARE_ACCOUNT_ID'),
  ]),
  tpl('vercel', 'Vercel', 'cloud', ['deploy'], [s('VERCEL_TOKEN')]),
  tpl('github', 'GitHub', 'cloud', ['git', 'ci', 'actions'], [s('GITHUB_TOKEN')]),

  // ── Auth ──
  tpl('jwt', 'JWT', 'auth', ['token'], [s('JWT_SECRET'), p('JWT_EXPIRES_IN', '15m')]),
  tpl('authjs', 'Auth.js', 'auth', ['nextauth', 'next-auth', 'oauth'], [
    s('AUTH_SECRET'), p('AUTH_URL', 'http://localhost:3000'),
  ]),
  tpl('auth0', 'Auth0', 'auth', ['oauth', 'oidc'], [
    p('AUTH0_DOMAIN'), p('AUTH0_CLIENT_ID'), s('AUTH0_CLIENT_SECRET'), s('AUTH0_SECRET'),
    p('APP_BASE_URL', 'http://localhost:3000'),
  ]),
  tpl('clerk', 'Clerk', 'auth', ['oauth'], [
    p('NEXT_PUBLIC_CLERK_PUBLISHABLE_KEY'), s('CLERK_SECRET_KEY'),
  ]),
  tpl('google-oauth', 'Google OAuth', 'auth', ['oauth', 'google', 'sso'], [
    p('GOOGLE_CLIENT_ID'), s('GOOGLE_CLIENT_SECRET'),
  ]),

  // ── Payments & messaging ──
  tpl('stripe', 'Stripe', 'payments', ['billing'], [
    s('STRIPE_SECRET_KEY'), p('STRIPE_PUBLISHABLE_KEY'), s('STRIPE_WEBHOOK_SECRET'),
  ]),
  tpl('smtp', 'SMTP', 'payments', ['email', 'mail'], [
    p('SMTP_HOST', 'localhost'), p('SMTP_PORT', '587'), p('SMTP_USER'), s('SMTP_PASSWORD'),
    p('MAIL_FROM', 'no-reply@example.com'),
  ]),
  tpl('sendgrid', 'SendGrid', 'payments', ['email', 'mail'], [s('SENDGRID_API_KEY')]),
  tpl('resend', 'Resend', 'payments', ['email', 'mail'], [s('RESEND_API_KEY')]),
  tpl('twilio', 'Twilio', 'payments', ['sms'], [p('TWILIO_ACCOUNT_SID'), s('TWILIO_AUTH_TOKEN')]),
  tpl('slack', 'Slack', 'payments', ['bot', 'chat'], [s('SLACK_BOT_TOKEN'), s('SLACK_SIGNING_SECRET')]),

  // ── Observability ──
  tpl('sentry', 'Sentry', 'observability', ['errors', 'monitoring'], [
    p('SENTRY_DSN'), p('SENTRY_ENVIRONMENT', 'development'),
  ]),
  tpl('otel', 'OpenTelemetry', 'observability', ['tracing', 'otlp', 'metrics'], [
    p('OTEL_SERVICE_NAME', 'app'), p('OTEL_EXPORTER_OTLP_ENDPOINT', 'http://localhost:4318'),
  ]),
  tpl('datadog', 'Datadog', 'observability', ['apm', 'monitoring'], [
    s('DD_API_KEY'), p('DD_SITE', 'datadoghq.com'), p('DD_ENV', 'dev'),
  ]),
];

const BY_ID = new Map(PROJECT_TEMPLATES.map((t) => [t.id, t]));

export function getTemplate(id: string): TemplateDef | undefined {
  return BY_ID.get(id);
}

/** Case-insensitive filter over label, id, group and keywords. */
export function searchTemplates(query: string, all: TemplateDef[] = PROJECT_TEMPLATES): TemplateDef[] {
  const q = query.trim().toLowerCase();
  if (!q) return all;
  return all.filter((t) =>
    [t.label, t.id, t.group, ...t.keywords].some((f) => f.toLowerCase().includes(q)));
}

export interface MergedVar {
  key:       string;
  value:     string;
  sensitive: boolean;
  /** Template id that first introduced the key (selection order). */
  source:    string;
  /** Other selected templates that also define this key. */
  alsoIn:    string[];
}

/** Merges selected templates' vars in selection order; first definition of a key wins. */
export function mergeTemplateVars(ids: string[]): MergedVar[] {
  const out: MergedVar[] = [];
  const byKey = new Map<string, MergedVar>();
  for (const id of ids) {
    const t = BY_ID.get(id);
    if (!t) continue;
    for (const v of t.vars) {
      const existing = byKey.get(v.key);
      if (existing) {
        if (!existing.alsoIn.includes(id)) existing.alsoIn.push(id);
        continue;
      }
      const merged: MergedVar = { key: v.key, value: v.example, sensitive: v.sensitive, source: id, alsoIn: [] };
      byKey.set(v.key, merged);
      out.push(merged);
    }
  }
  return out;
}

/** Deduplicated category names for the selected templates, in selection order. */
export function templateCategories(ids: string[]): string[] {
  const out: string[] = [];
  for (const id of ids) {
    const c = BY_ID.get(id)?.category;
    if (c && !out.includes(c)) out.push(c);
  }
  return out;
}

/** Value persisted in `projects.template`: comma-joined ids, `generic` when none. */
export function templateString(ids: string[]): string {
  return ids.length > 0 ? ids.join(',') : 'generic';
}

/** Human labels for a stored template string; unknown ids are shown raw. */
export function templateLabels(stored: string): string[] {
  return stored.split(',').map((id) => id.trim()).filter(Boolean)
    .map((id) => BY_ID.get(id)?.label ?? id);
}

/** Normalises a user-typed key to the backend's `[A-Z_][A-Z0-9_]*` alphabet. */
export function normalizeEnvKey(raw: string): string {
  return raw.toUpperCase().replace(/[^A-Z0-9_]/g, '');
}

export function isValidEnvKey(key: string): boolean {
  return /^[A-Z_][A-Z0-9_]*$/.test(key);
}
