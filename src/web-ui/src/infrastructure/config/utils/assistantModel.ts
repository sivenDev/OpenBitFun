import { configManager } from '../services/ConfigManager';
import type { AIModelConfig, DefaultModelsConfig } from '../types';

/** Default models chosen on assistant home pages, keyed by assistant ID. */
export const ASSISTANT_DEFAULT_MODELS_PATH = 'app.assistant_default_models';

export type AssistantModelSource = 'assistant' | 'primary' | 'chosenUnavailable';

export interface AssistantModel {
  /** The enabled model the assistant runs on, or `null` without a primary model. */
  model: AIModelConfig | null;
  source: AssistantModelSource;
  /** The model chosen for the assistant, kept while it is unavailable. */
  chosenId: string | null;
}

function enabledModel(models: readonly AIModelConfig[], id: string | null | undefined) {
  if (!id) return null;
  const matches = models.filter(model => model.enabled && model.id === id);
  return matches.length === 1 ? matches[0] : null;
}

/**
 * The model an assistant runs on, as the host resolves it: its own default
 * while that model is enabled, else the primary model. The last model picked
 * in a chat never decides.
 */
export function resolveAssistantModel(
  assistantId: string,
  preferences: Readonly<Record<string, string>> | undefined,
  models: readonly AIModelConfig[],
  defaults: DefaultModelsConfig | null | undefined,
): AssistantModel {
  const chosenId = preferences?.[assistantId]?.trim() || null;
  const chosen = enabledModel(models, chosenId);
  if (chosen) return { model: chosen, source: 'assistant', chosenId };
  return {
    model: enabledModel(models, defaults?.primary),
    source: chosenId ? 'chosenUnavailable' : 'primary',
    chosenId,
  };
}

/** Chooses an assistant's default model; `null` follows the primary model again. */
export async function saveAssistantDefaultModel(assistantId: string, modelId: string | null): Promise<void> {
  await configManager.updateConfig<Record<string, string>>(ASSISTANT_DEFAULT_MODELS_PATH, current => {
    const next = { ...current };
    if (modelId) next[assistantId] = modelId;
    else delete next[assistantId];
    return next;
  });
}
