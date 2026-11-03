/**
 * Says why a conversation with a shared assistant cannot go on, and what to
 * do next, in place of the backend's marked error.
 */

import { i18nService } from '@/infrastructure/i18n';
import { sharedAssistantProblem, type SharedAssistantProblem } from './types';

/** A shared-assistant failure whose message is ready to show. */
export class SharedAssistantError extends Error {
  constructor(readonly sharedAssistantProblem: SharedAssistantProblem, message: string) {
    super(message);
    this.name = 'SharedAssistantError';
  }
}

export function sharedAssistantProblemText(problem: SharedAssistantProblem, name: string): string {
  return i18nService.t(`flow-chat:chatInput.dispatch.sharedAssistant.problems.${problem}`, { name });
}

/**
 * `error` with a message to show when it is a shared-assistant failure;
 * any other error unchanged.
 */
export function explainSharedAssistantError(error: unknown, name: string): unknown {
  if (error instanceof SharedAssistantError) return error;
  const problem = sharedAssistantProblem(error);
  return problem ? new SharedAssistantError(problem, sharedAssistantProblemText(problem, name)) : error;
}
