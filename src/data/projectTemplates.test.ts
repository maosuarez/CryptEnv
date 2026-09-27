import { describe, expect, it } from 'vitest';
import {
  PROJECT_TEMPLATES, TEMPLATE_GROUPS, TEMPLATE_PLACEHOLDER, isValidEnvKey, mergeTemplateVars,
  normalizeEnvKey, searchTemplates, templateCategories, templateLabels, templateString,
} from './projectTemplates';

describe('project template catalog', () => {
  it('has unique ids and at least 50 templates', () => {
    const ids = PROJECT_TEMPLATES.map((t) => t.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(PROJECT_TEMPLATES.length).toBeGreaterThanOrEqual(50);
  });

  it('covers every group', () => {
    for (const g of TEMPLATE_GROUPS) {
      expect(PROJECT_TEMPLATES.some((t) => t.group === g)).toBe(true);
    }
  });

  it('includes the required stacks', () => {
    for (const id of ['postgres', 'mysql', 'mongo', 'redis', 'node', 'python', 'java', 'spring', 'docker', 'openai', 'anthropic', 'gemini']) {
      expect(PROJECT_TEMPLATES.some((t) => t.id === id)).toBe(true);
    }
  });

  it('uses valid, non-duplicated keys per template', () => {
    for (const t of PROJECT_TEMPLATES) {
      expect(t.vars.length).toBeGreaterThan(0);
      const keys = t.vars.map((v) => v.key);
      expect(new Set(keys).size, t.id).toBe(keys.length);
      for (const k of keys) expect(isValidEnvKey(k), `${t.id}:${k}`).toBe(true);
    }
  });

  it('never ships a real-looking credential in sensitive examples', () => {
    for (const t of PROJECT_TEMPLATES) {
      for (const v of t.vars.filter((x) => x.sensitive)) {
        expect(v.example === '' || v.example.includes(TEMPLATE_PLACEHOLDER), `${t.id}:${v.key}`).toBe(true);
      }
    }
  });
});

describe('searchTemplates', () => {
  it('matches label, id, group and keywords case-insensitively', () => {
    const ids = searchTemplates('SQL').map((t) => t.id);
    expect(ids).toEqual(expect.arrayContaining(['postgres', 'mysql', 'sqlserver', 'sqlite']));
    expect(ids).not.toContain('openai');
    expect(searchTemplates('claude').map((t) => t.id)).toEqual(['anthropic']);
  });

  it('returns everything for an empty query and nothing for gibberish', () => {
    expect(searchTemplates('  ')).toHaveLength(PROJECT_TEMPLATES.length);
    expect(searchTemplates('zzzqqq')).toEqual([]);
  });
});

describe('mergeTemplateVars', () => {
  it('dedups shared keys, first template wins', () => {
    const merged = mergeTemplateVars(['prisma', 'postgres']);
    const db = merged.filter((v) => v.key === 'DATABASE_URL');
    expect(db).toHaveLength(1);
    expect(db[0].source).toBe('prisma');
    expect(db[0].alsoIn).toEqual(['postgres']);
    expect(merged[0].key).toBe('DATABASE_URL');
  });

  it('keeps selection order and ignores unknown ids', () => {
    const keys = mergeTemplateVars(['redis', 'nope', 'node']).map((v) => v.key);
    expect(keys).toEqual(['REDIS_URL', 'REDIS_PASSWORD', 'NODE_ENV', 'PORT', 'LOG_LEVEL']);
    expect(mergeTemplateVars([])).toEqual([]);
  });
});

describe('template helpers', () => {
  it('derives deduplicated categories', () => {
    expect(templateCategories(['node', 'postgres', 'openai', 'node'])).toEqual(['Node.js', 'PostgreSQL', 'OpenAI']);
  });

  it('serialises and labels the template string', () => {
    expect(templateString([])).toBe('generic');
    expect(templateString(['node', 'postgres'])).toBe('node,postgres');
    expect(templateLabels('node,postgres')).toEqual(['Node.js', 'PostgreSQL']);
    expect(templateLabels('legacy-thing')).toEqual(['legacy-thing']);
  });

  it('normalises keys', () => {
    expect(normalizeEnvKey('my-key.v2')).toBe('MYKEYV2');
    expect(isValidEnvKey('1ABC')).toBe(false);
    expect(isValidEnvKey('_OK_1')).toBe(true);
  });
});
