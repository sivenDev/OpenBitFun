import type { SharedWithMe } from '@/infrastructure/api/service-api/LanShareAPI';
import { workspaceManager } from '@/infrastructure/services/business/workspaceManager';
import { resolveSubagentNameKey } from '../subagent-identity';
import type { Session } from '../types/flow-chat';
import { calledAssistantLabels } from './remoteAgent';

/**
 * The name a child agent session shows: the called assistant's own name when
 * the session is a call to an assistant, otherwise the generated nickname.
 */
export function subagentDisplayName(
  t: (key: string) => string,
  sessionId: string,
  session: Pick<Session, 'subagentType' | 'config' | 'delegatedBy' | 'workspaceId'> | null | undefined,
  sharedWithMe: readonly SharedWithMe[],
): string {
  const labels = calledAssistantLabels(
    session?.subagentType || session?.config?.agentType,
    session,
    workspaceManager.getState().assistantWorkspaces,
    sharedWithMe,
  );
  if (labels?.name) return labels.name;
  if (labels?.owner.kind === 'shared') return t('flowChatHeader.agentTreeSharedAssistant');
  return t(resolveSubagentNameKey(sessionId));
}
