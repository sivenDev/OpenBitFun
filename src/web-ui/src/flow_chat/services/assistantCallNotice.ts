import type { SharedWithMe } from '@/infrastructure/api/service-api/LanShareAPI';
import type { FlowChatState } from '../types/flow-chat';
import { remoteAgentFromAgentType } from '../utils/remoteAgent';
import { resolveSessionTitle } from '../utils/sessionTitle';
import { subagentDisplayName } from '../utils/subagentDisplayName';

export type AssistantCallOutcome = 'completed' | 'failed' | 'cancelled';

export interface AssistantCallNotice {
  callerSessionId: string;
  message: string;
}

/**
 * What to tell the user when a call to an assistant ends while they are not
 * in the conversation that made it: the call card there already shows the
 * outcome. Null for any other session, or when the user is looking at the
 * caller or the caller is itself running and will read the result.
 */
export function assistantCallNotice(
  state: Pick<FlowChatState, 'sessions'>,
  sessionId: string,
  outcome: AssistantCallOutcome,
  options: {
    /** Translates `flow-chat` keys. */
    t: (key: string, values?: Record<string, unknown>) => string;
    /** Translates namespaced keys. */
    translate: (key: string) => string;
    sharedWithMe: readonly SharedWithMe[];
    callerIsRunning: (sessionId: string) => boolean;
    callerIsVisible: (sessionId: string) => boolean;
  },
): AssistantCallNotice | null {
  const call = state.sessions.get(sessionId);
  if (!call) return null;
  const callType = call.subagentType || call.config?.agentType;
  // A local call is a session of the called assistant's own; a call to a
  // shared assistant is a child of the caller's session.
  const callerSessionId = call.delegatedBy?.sessionId
    ?? (remoteAgentFromAgentType(callType) ? call.parentSessionId : undefined);
  if (!callerSessionId) return null;
  const caller = state.sessions.get(callerSessionId);
  if (!caller || options.callerIsVisible(callerSessionId) || options.callerIsRunning(callerSessionId)) {
    return null;
  }
  const conversation = resolveSessionTitle(caller, options.translate);
  return {
    callerSessionId,
    message: options.t(`assistantCallNotice.${outcome}`, {
      agent: subagentDisplayName(options.t, sessionId, call, options.sharedWithMe),
      conversation,
    }),
  };
}
