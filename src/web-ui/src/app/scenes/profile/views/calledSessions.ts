import type { Session } from '@/flow_chat/types/flow-chat';

/** The conversations in `workspaceId` that another assistant started by calling this one. */
export function calledSessionsIn(
  sessions: Iterable<Session>,
  workspaceId: string | undefined,
): Session[] {
  if (!workspaceId) return [];
  return Array.from(sessions)
    .filter(session => session.workspaceId === workspaceId && session.delegatedBy)
    .sort((left, right) => (right.lastActiveAt ?? 0) - (left.lastActiveAt ?? 0));
}
