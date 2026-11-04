/**
 * Why a conversation with a shared assistant cannot go on, for the turn
 * failure notice, in place of a model failure.
 */

import { sharedAssistantProblem } from '@/features/dispatch/types';
import { sharedAssistantProblemText } from '@/features/dispatch/sharedAssistantProblemText';
import type { Session } from '../../types/flow-chat';

export function sharedAssistantFailureText(
  session: Pick<Session, 'config'> | null | undefined,
  error: string,
): string | undefined {
  const target = session?.config.dispatchTarget;
  if (target?.kind !== 'sharedAssistant') return undefined;
  const problem = sharedAssistantProblem(error);
  return problem ? sharedAssistantProblemText(problem, target.displayName) : undefined;
}
