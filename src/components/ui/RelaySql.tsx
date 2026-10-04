import { Icon } from './Icon';
import { useTranslation } from '../../i18n';

/** Relay SQL v2: no anon table access; single-use and expiry enforced by the RPCs. */
export const RELAY_SQL = `create table if not exists relay_packages_v2 (
  code_hash  text primary key,
  payload    text not null check (octet_length(payload) <= 1048576),
  expires_at timestamptz not null default now() + interval '24 hours'
);
alter table relay_packages_v2 enable row level security;
revoke all on relay_packages_v2 from anon, authenticated;

create or replace function relay_put(p_code_hash text, p_payload text) returns void
language plpgsql security definer set search_path = public as $$
begin
  delete from relay_packages_v2 where expires_at < now();
  insert into relay_packages_v2(code_hash, payload) values (p_code_hash, p_payload);
end $$;

create or replace function relay_claim(p_code_hash text) returns text
language plpgsql security definer set search_path = public as $$
declare v text;
begin
  delete from relay_packages_v2 where expires_at < now();
  delete from relay_packages_v2 where code_hash = p_code_hash returning payload into v;
  return v;
end $$;

create or replace function relay_schema_version() returns int language sql as $$ select 2 $$;
grant execute on function relay_put(text,text), relay_claim(text), relay_schema_version() to anon;`;

/** Marker the backend puts at the start of the RelaySchemaOutdated error. */
const OUTDATED_MARKER = 'RELAY_SCHEMA_OUTDATED';

export function isRelaySchemaOutdated(msg: string): boolean {
  return msg.includes(OUTDATED_MARKER);
}

/** Copyable SQL block, shared by Settings and the send/receive error notice. */
export function RelaySqlBlock({ onCopied }: { onCopied?: () => void }) {
  const { t } = useTranslation();
  return (
    <div className="rounded-[3px] border border-bd bg-raised overflow-hidden">
      <div className="flex items-center justify-between px-3 py-2 border-b border-bd">
        <span className="text-[11px] font-mono text-tx3 tracking-[0.06em]">{t('settings.relay.runInEditor')}</span>
        <button
          onClick={() => navigator.clipboard.writeText(RELAY_SQL).then(() => onCopied?.())}
          className="text-tx3 hover:text-accent transition-colors"
          title={t('settings.relay.copySql')}
          aria-label={t('settings.relay.copySql')}
        >
          <Icon name="copy" size={12} />
        </button>
      </div>
      <pre className="text-[11px] font-mono text-tx2 p-4 overflow-x-auto leading-[1.6] whitespace-pre-wrap">{RELAY_SQL}</pre>
    </div>
  );
}

/** Error notice shown in send/receive modals when the relay still has the v1 schema. */
export function RelaySchemaOutdatedNotice() {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col gap-2">
      <div className="text-[12px] text-danger font-mono bg-danger-b border border-danger rounded-[3px] px-3 py-2">
        {t('settings.relay.schemaOutdated')}
      </div>
      <RelaySqlBlock />
    </div>
  );
}
