/**
 * Follows the session of a task another device or account delegated to an
 * assistant here. The runner that accepted the task owns its session and
 * event log on this device; the owner's view replays that log through the
 * same projection Dispatch observers use, so the session reads like any
 * other, but it is never persisted or driven from this window.
 */

import { createLogger } from '@/shared/utils/logger';
import { delegationAPI } from '@/infrastructure/api/service-api/DelegationAPI';
import { agenticEventListener } from '../../services/AgenticEventListener';
import type { FlowChatContext } from '../../services/flow-chat-manager/types';
import { dispatchEventId, projectDispatchAgentEvent } from '@/features/dispatch/DispatchJobObserver';
import { isDispatchJobTerminal, type DispatchJobState } from '@/features/dispatch/types';
import { delegatedSessionEntry, delegatedSessionStore } from './delegatedSessionStore';

const log = createLogger('DelegatedTaskFollower');

/** How often a running task is read while nothing new arrived. */
export const DELEGATED_FOLLOW_INTERVAL_MS = 2_000;
/** How long to wait after a failed read before trying again. */
const DELEGATED_FOLLOW_RETRY_MS = 5_000;

export interface DelegatedTaskToFollow {
  jobId: string;
  sessionId: string;
  title: string;
  state: string;
  /** The assistant's workspace, so the session opens beside its own. */
  workspacePath: string;
  workspaceId?: string;
}

let followerContext: FlowChatContext | null = null;
const following = new Set<string>();
const appliedEventIds = new Map<string, Set<string>>();

const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));

export function installDelegatedTaskFollower(context: FlowChatContext): () => void {
  followerContext = context;
  return () => {
    if (followerContext === context) followerContext = null;
  };
}

/**
 * Make the task's session available in this window and keep it current
 * until the task has finished and its log is fully replayed.
 */
export function followDelegatedTask(task: DelegatedTaskToFollow): boolean {
  const context = followerContext;
  if (!context || !task.sessionId) return false;
  delegatedSessionStore.getState().register({
    jobId: task.jobId,
    sessionId: task.sessionId,
    cursor: 0,
    state: task.state,
    pendingPermissions: [],
  });
  context.flowChatStore.addExternalSession(
    task.sessionId,
    task.title,
    'Claw',
    task.workspacePath,
    { projectWorkspacePath: task.workspacePath, workspaceId: task.workspaceId },
  );
  void follow(context, task.sessionId);
  return true;
}

/** Stop following a session the owner closed here; the task itself runs on. */
export function forgetDelegatedTask(sessionId: string): void {
  delegatedSessionStore.getState().remove(sessionId);
  appliedEventIds.delete(sessionId);
}

async function follow(context: FlowChatContext, sessionId: string): Promise<void> {
  if (following.has(sessionId)) return;
  following.add(sessionId);
  try {
    for (;;) {
      const entry = delegatedSessionEntry(sessionId);
      if (!entry || followerContext !== context || !context.flowChatStore.getState().sessions.has(sessionId)) {
        return;
      }
      let response;
      try {
        response = await delegationAPI.taskEvents(entry.jobId, entry.cursor);
      } catch (error) {
        log.warn('Failed to read a delegated task', { jobId: entry.jobId, error });
        await sleep(DELEGATED_FOLLOW_RETRY_MS);
        continue;
      }
      const applied = appliedEventIds.get(sessionId) ?? new Set<string>();
      appliedEventIds.set(sessionId, applied);
      for (const event of response.events) {
        const eventId = dispatchEventId(event);
        if (applied.has(eventId)) continue;
        if (event.type === 'agentEvent') {
          const projected = projectDispatchAgentEvent(event);
          if (projected) agenticEventListener.dispatchExternal(projected.eventName, projected.payload);
        }
        applied.add(eventId);
      }
      context.eventBatcher.flushNow();
      delegatedSessionStore.getState().update(sessionId, {
        cursor: response.cursor,
        state: response.state,
        pendingPermissions: response.pendingPermissions ?? [],
      });
      const drained = isDispatchJobTerminal(response.state as DispatchJobState)
        && response.events.length === 0
        && response.cursor === entry.cursor;
      if (drained) return;
      if (response.events.length === 0) await sleep(DELEGATED_FOLLOW_INTERVAL_MS);
    }
  } finally {
    following.delete(sessionId);
  }
}
