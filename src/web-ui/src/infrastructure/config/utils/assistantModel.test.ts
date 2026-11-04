import { describe, expect, it, vi } from 'vitest';
import type { AIModelConfig } from '../types';

const mocks = vi.hoisted(() => ({
  stored: {} as Record<string, string>,
}));

vi.mock('../services/ConfigManager', () => ({
  configManager: {
    updateConfig: vi.fn(async (_path: string, update: (current: Record<string, string>) => Record<string, string>) => {
      mocks.stored = update(mocks.stored);
      return mocks.stored;
    }),
  },
}));

import { resolveAssistantModel, saveAssistantDefaultModel } from './assistantModel';

const model = (id: string, enabled = true) => ({ id, model_name: id, enabled }) as AIModelConfig;

describe('resolveAssistantModel', () => {
  const models = [model('sol'), model('terra'), model('luna', false)];
  const defaults = { primary: 'sol' };

  it('runs an assistant on its own model else the primary model', () => {
    expect(resolveAssistantModel('xiaolan', {}, models, defaults))
      .toEqual({ model: models[0], source: 'primary', chosenId: null });
    expect(resolveAssistantModel('xiaolan', undefined, models, defaults).source).toBe('primary');
    expect(resolveAssistantModel('xiaolan', { xiaolan: 'terra' }, models, defaults))
      .toEqual({ model: models[1], source: 'assistant', chosenId: 'terra' });
    expect(resolveAssistantModel('xiaoqi', { xiaolan: 'terra' }, models, defaults).source).toBe('primary');
  });

  it('keeps an unavailable choice and runs on the primary model meanwhile', () => {
    for (const chosen of ['luna', 'removed']) {
      expect(resolveAssistantModel('xiaolan', { xiaolan: chosen }, models, defaults))
        .toEqual({ model: models[0], source: 'chosenUnavailable', chosenId: chosen });
    }
    expect(resolveAssistantModel('xiaolan', {}, models, { primary: null }).model).toBeNull();
  });
});

describe('saveAssistantDefaultModel', () => {
  it('sets a choice and removes it to follow the primary model again', async () => {
    mocks.stored = { other: 'sol' };
    await saveAssistantDefaultModel('xiaolan', 'terra');
    expect(mocks.stored).toEqual({ other: 'sol', xiaolan: 'terra' });
    await saveAssistantDefaultModel('xiaolan', null);
    expect(mocks.stored).toEqual({ other: 'sol' });
  });
});
