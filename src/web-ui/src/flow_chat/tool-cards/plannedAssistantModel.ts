import type { AssistantModel } from '@/infrastructure/config/utils/assistantModel';

/**
 * The model a local assistant will run on, for cards shown before it runs:
 * the concrete model name, marked when it is the primary model.
 */
export function plannedAssistantModelLabel(
  planned: AssistantModel | null | undefined,
  t: (key: string, options?: Record<string, unknown>) => string,
): string | null {
  const model = planned?.model;
  if (!model) return null;
  const name = model.model_name || model.name || model.id || '';
  if (!name) return null;
  return planned.source === 'assistant'
    ? name
    : t('toolCards.taskTool.assistantModelPrimary', { name });
}
