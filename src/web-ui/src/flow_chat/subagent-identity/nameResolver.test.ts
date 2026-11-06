import { describe, expect, it } from 'vitest';
import { SHARED_TERMS_BY_LOCALE } from '@/infrastructure/i18n/presets/generatedLocaleContract';
import { resolveSubagentNameKey, SUBAGENT_NAME_KEYS } from './nameResolver';

const en = SHARED_TERMS_BY_LOCALE['en-US'].subagentNames;
const zh = SHARED_TERMS_BY_LOCALE['zh-CN'].subagentNames;
const tw = SHARED_TERMS_BY_LOCALE['zh-TW'].subagentNames;

describe('localized subagent names', () => {
  it('provides exactly 30 distinct identities in every supported language', () => {
    expect(SUBAGENT_NAME_KEYS).toHaveLength(30);
    expect(new Set(SUBAGENT_NAME_KEYS).size).toBe(30);
    for (const names of [en, zh, tw]) {
      expect(Object.keys(names)).toHaveLength(30);
      expect(new Set(Object.values(names)).size).toBe(30);
      for (const key of SUBAGENT_NAME_KEYS) {
        expect(names[key.split('.').at(-1) as keyof typeof names].trim()).not.toBe('');
      }
    }
  });

  it('keeps a restored session identity independent of locale and lookup order', () => {
    expect(resolveSubagentNameKey('child-session')).toBe('shared:subagentNames.cloudHopper');
    resolveSubagentNameKey('another-session');
    expect(resolveSubagentNameKey(' child-session ')).toBe('shared:subagentNames.cloudHopper');
    expect(zh.cloudHopper).not.toBe(en.cloudHopper);
    expect(resolveSubagentNameKey('   ')).toBe(SUBAGENT_NAME_KEYS[0]);
  });

  // Core resolves the same names for the model; its tests pin these vectors too.
  it('matches the names Core resolves for the same sessions', () => {
    expect(resolveSubagentNameKey('a3ccf64c-8a61-4281-9d87-7fdb78f13fa7')).toBe('shared:subagentNames.captainDucky');
    expect(resolveSubagentNameKey('875406c8-de4b-4906-a4f7-82c43436a88d')).toBe('shared:subagentNames.jellyRocket');
    expect(resolveSubagentNameKey('child-session')).toBe('shared:subagentNames.cloudHopper');
  });
});
