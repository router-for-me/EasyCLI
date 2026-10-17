import { describe, expect, test } from 'bun:test';
import {
  cloneCodexModelConfiguration,
  codexContextSourceHint,
  sameCodexModelConfiguration,
  toggleCodexReasoningLevel,
  validateCodexModelConfiguration,
  type CodexModelConfiguration,
  type CodexCatalogEditorModel,
} from '../src/services/codexModelCatalog';

const configuration = (): CodexModelConfiguration => ({
  display_name: 'Third Party',
  description: null,
  base_instructions: 'You are a helpful coding assistant.',
  context_window: 128_000,
  max_context_window: 128_000,
  effective_context_window_percent: 95,
  auto_compact_token_limit: null,
  default_reasoning_level: 'medium',
  supported_reasoning_levels: [
    { effort: 'low', description: 'Low' },
    { effort: 'medium', description: 'Medium' },
    { effort: 'high', description: 'High' },
  ],
  input_modalities: ['text', 'image'],
  visibility: 'list',
  supports_parallel_tool_calls: false,
  supports_search_tool: false,
});

describe('Codex 模型列表编辑', () => {
  test('仅修改 Tool Search 也能触发保存，并可恢复默认值', () => {
    const original = configuration();
    const edited = cloneCodexModelConfiguration(original);
    edited.supports_search_tool = true;
    expect(sameCodexModelConfiguration(original, edited)).toBeFalse();
    expect(validateCodexModelConfiguration(edited)).toBeNull();
    expect(original.supports_search_tool).toBeFalse();
    expect(sameCodexModelConfiguration(original, cloneCodexModelConfiguration(original))).toBeTrue();
  });
  test('识别系统提示词修改并拒绝空提示词', () => {
    const original = configuration();
    const edited = { ...original, base_instructions: '请用中文回答。\n保留换行。' };
    expect(sameCodexModelConfiguration(original, edited)).toBeFalse();
    expect(validateCodexModelConfiguration(edited)).toBeNull();
    expect(validateCodexModelConfiguration({ ...original, base_instructions: ' \n ' }))
      .toBe('agents.catalog.invalidPrompt');
  });
  test("区分 Codex 客户端 API、模板后备与自定义上下文", () => {
    const model: CodexCatalogEditorModel = {
      slug: "model-a", hasOfficialTemplate: true, customized: false,
      contextSource: "client", configuration: configuration(), defaults: configuration(),
    };
    expect(codexContextSourceHint(model)).toBe("agents.catalog.contextSource.client");
    expect(codexContextSourceHint({ ...model, contextSource: "template" }))
      .toBe("agents.catalog.contextSource.template");
    model.configuration.context_window = 64_000;
    expect(codexContextSourceHint(model)).toBe("agents.catalog.contextSource.customized");
    model.configuration = cloneCodexModelConfiguration(model.defaults);
    expect(codexContextSourceHint(model)).toBe("agents.catalog.contextSource.client");
  });

  test('克隆配置时隔离数组字段', () => {
    const original = configuration();
    const cloned = cloneCodexModelConfiguration(original);
    cloned.supported_reasoning_levels[0].description = 'Changed';
    cloned.input_modalities.pop();

    expect(original.supported_reasoning_levels[0].description).toBe('Low');
    expect(original.input_modalities).toEqual(['text', 'image']);
    expect(sameCodexModelConfiguration(original, cloned)).toBeFalse();
  });

  test('移除默认思考等级时选择剩余可用等级', () => {
    const updated = toggleCodexReasoningLevel(configuration(), 'medium', false, []);
    expect(updated.supported_reasoning_levels.map((level) => level.effort)).toEqual(['low', 'high']);
    expect(updated.default_reasoning_level).toBe('low');
  });

  test('校验上下文、压缩阈值和输入类型', () => {
    expect(validateCodexModelConfiguration(configuration())).toBeNull();
    expect(validateCodexModelConfiguration({ ...configuration(), max_context_window: 64_000 }))
      .toBe('agents.catalog.invalidMaximum');
    expect(validateCodexModelConfiguration({ ...configuration(), auto_compact_token_limit: 200_000 }))
      .toBe('agents.catalog.invalidCompact');
    expect(validateCodexModelConfiguration({ ...configuration(), input_modalities: [] }))
      .toBe('agents.catalog.invalidModalities');
  });
});
