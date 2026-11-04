import { useEffect, useMemo, useState } from 'react';
import { configManager } from '../services/ConfigManager';
import type { AIModelConfig, DefaultModelsConfig } from '../types';
import {
  ASSISTANT_DEFAULT_MODELS_PATH,
  resolveAssistantModel,
  type AssistantModel,
} from '../utils/assistantModel';
import { createLogger } from '@/shared/utils/logger';

const log = createLogger('useAssistantModel');

const WATCHED_PATHS = ['ai.models', 'ai.default_models', ASSISTANT_DEFAULT_MODELS_PATH];

interface AssistantModelConfig {
  models: AIModelConfig[];
  defaults: DefaultModelsConfig | null;
  /** `undefined` on hosts that predate assistant default models. */
  preferences: Record<string, string> | undefined;
}

function cachedConfig(): AssistantModelConfig {
  return {
    models: configManager.get<AIModelConfig[] | null>('ai.models', null) ?? [],
    defaults: configManager.get<DefaultModelsConfig | null>('ai.default_models', null),
    preferences: configManager.get<Record<string, string> | undefined>(ASSISTANT_DEFAULT_MODELS_PATH),
  };
}

function affects(changed: string) {
  return WATCHED_PATHS.some(path => (
    changed === path || path.startsWith(`${changed}.`) || changed.startsWith(`${path}.`)
  ));
}

export interface UseAssistantModelResult extends AssistantModel {
  /** Whether this host stores assistant default models. */
  supported: boolean;
  models: AIModelConfig[];
  primaryModel: AIModelConfig | null;
}

/** The model an assistant runs on, following model and assistant settings. */
export function useAssistantModel(assistantId: string | null | undefined): UseAssistantModelResult | null {
  const [config, setConfig] = useState<AssistantModelConfig>(cachedConfig);

  useEffect(() => {
    let cancelled = false;
    const load = () => {
      void Promise.all([
        configManager.getConfig<AIModelConfig[]>('ai.models'),
        configManager.getConfig<DefaultModelsConfig>('ai.default_models'),
        // Older hosts have no such setting; their assistants follow the primary model.
        configManager.getConfig<Record<string, string> | undefined>(ASSISTANT_DEFAULT_MODELS_PATH)
          .catch(() => undefined),
      ]).then(([models, defaults, preferences]) => {
        if (!cancelled) setConfig({ models: models ?? [], defaults: defaults ?? null, preferences });
      }).catch(error => {
        log.warn('Failed to read assistant model settings', error);
      });
    };
    load();
    const unsubscribe = configManager.onConfigChange(path => {
      if (affects(path)) load();
    });
    return () => {
      cancelled = true;
      unsubscribe();
    };
  }, []);

  return useMemo(() => {
    if (!assistantId) return null;
    const resolved = resolveAssistantModel(assistantId, config.preferences, config.models, config.defaults);
    const primaryModel = resolveAssistantModel('', undefined, config.models, config.defaults).model;
    return {
      ...resolved,
      supported: config.preferences !== undefined,
      models: config.models,
      primaryModel,
    };
  }, [assistantId, config]);
}
