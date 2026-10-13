import { describe, expect, test } from 'bun:test';
import {
  combineModelAliasEntries,
  combineModelAliasSources,
  defaultModelAlias,
  thinkingAliasSourceKindLabel,
  uniqueModelAlias,
} from '../src/pages/ThinkingAliasesPage';

describe('模型别名默认名称', () => {
  test('按思考强度和 Fast 选项生成可编辑的默认名称', () => {
    expect(defaultModelAlias('gpt-5.6-sol', 'XHigh', false)).toBe('gpt-5.6-sol-xhigh');
    expect(defaultModelAlias('gpt-5.6-sol', '', true)).toBe('gpt-5.6-sol-fast');
    expect(defaultModelAlias('gpt-5.6-sol', 'xhigh', true)).toBe('gpt-5.6-sol-xhigh-fast');
    expect(defaultModelAlias('gpt-5.6-sol', '', false)).toBe('gpt-5.6-sol-alias');
  });

  test('默认别名与现有模型重名时递增数字后缀', () => {
    expect(uniqueModelAlias('gpt-5.6-sol-high', ['gpt-5.6-sol-high'])).toBe('gpt-5.6-sol-high-2');
    expect(uniqueModelAlias('gpt-5.6-sol-high', [
      'gpt-5.6-sol-high',
      'gpt-5.6-sol-high-2',
    ])).toBe('gpt-5.6-sol-high-3');
    expect(uniqueModelAlias('gpt-5.6-sol-high', ['GPT-5.6-SOL-HIGH'])).toBe('gpt-5.6-sol-high-2');
  });
});

describe('统一模型别名列表', () => {
  test('同一个别名可同时显示思考强度和 Fast', () => {
    const identity = {
      position: { section: 'oauth-model-alias', providerIndex: null, modelIndex: 0 },
      sourceModel: 'gpt-5.6-sol',
      alias: 'gpt-5.6-sol-xhigh-fast',
      provider: 'Codex OAuth',
      kind: 'codex-oauth',
      oauthChannel: 'codex',
    };
    const entries = combineModelAliasEntries(
      [{ ...identity, effort: 'xhigh' }],
      [{ ...identity, serviceTier: 'priority' }],
    );

    expect(entries).toHaveLength(1);
    expect(entries[0]).toMatchObject({ effort: 'xhigh', serviceTier: 'priority' });
  });

  test('Fast-only 别名仍会显示为独立条目', () => {
    const entries = combineModelAliasEntries([], [{
      position: { section: 'oauth-model-alias', providerIndex: null, modelIndex: 0 },
      sourceModel: 'gpt-5.6-sol',
      alias: 'gpt-5.6-sol-fast',
      serviceTier: 'priority',
      provider: 'Codex OAuth',
      kind: 'codex-oauth',
      oauthChannel: 'codex',
    }]);

    expect(entries[0]).toMatchObject({ effort: null, serviceTier: 'priority' });
  });
});

describe('思考别名来源', () => {
  test('区分同名模型的接入来源', () => {
    expect(thinkingAliasSourceKindLabel('codex-oauth')).toBe('Codex OAuth');
    expect(thinkingAliasSourceKindLabel('claude-api')).toBe('Claude API');
    expect(thinkingAliasSourceKindLabel('openai-compatible')).toBe('OpenAI Compatible');
  });

  test('仅把内核报告了思考等级的来源标为可覆写', () => {
    const baseSource = {
      model: 'shared-model',
      displayName: 'Shared Model',
      provider: 'Provider',
      kind: 'codex-api',
      protocol: 'codex',
      reasoningLevels: [] as string[],
    };
    const sources = combineModelAliasSources(
      [
        { ...baseSource, id: 'reasoning' },
        { ...baseSource, id: 'plain', model: 'plain-model' },
      ],
      [{ ...baseSource, id: 'reasoning', reasoningLevels: ['low', 'high'] }],
      [{ ...baseSource, id: 'reasoning', reasoningLevels: ['low', 'high'] }],
    );

    expect(sources).toHaveLength(2);
    expect(sources.find((source) => source.id === 'reasoning')).toMatchObject({
      supportsReasoning: true,
      supportsFast: true,
      reasoningLevels: ['low', 'high'],
    });
    expect(sources.find((source) => source.id === 'plain')?.supportsReasoning).toBe(false);
    expect(sources.find((source) => source.id === 'plain')?.supportsFast).toBe(false);
  });
});
