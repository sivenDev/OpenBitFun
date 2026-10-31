/**
 * The owner's read-only view of a task another device or account delegated
 * to an assistant here. The task belongs to whoever sent it: the owner can
 * watch it and answer the actions it waits on, one at a time, but cannot
 * send messages, rename, or cancel it from the session.
 */

import { i18nService } from '@/infrastructure/i18n';
import { delegationAPI } from '@/infrastructure/api/service-api/DelegationAPI';
import type { FlowChatContext } from '../../services/flow-chat-manager/types';
import { cleanupSaveState } from '../../services/flow-chat-manager/PersistenceModule';
import { cleanupSessionBuffers } from '../../services/flow-chat-manager/TextChunkModule';
import { selectActivePermissionBatch } from '../../components/modern/permissionRequestRouting';
import type { PermissionRequest } from '@/infrastructure/api/service-api/AgentAPI';
import type {
  PermissionReplyKindLike,
  PermissionRequestLike,
  SessionCascadeRemoval,
  SessionDriver,
} from '../types';
import { resolveDelegatedSessionId } from '../resolve';
import { delegatedSessionEntry, delegatedSessionStore } from './delegatedSessionStore';
import { forgetDelegatedTask } from './delegatedTaskFollower';

const readOnlyReason = () => i18nService.t('flow-chat:chatInput.delegated.readOnly');
const readOnlyError = () => new Error(readOnlyReason());

/** Requests answered here that the runner still lists until it consumes the answer. */
const answeredRequestIds = new Map<string, Set<string>>();
const EMPTY: readonly PermissionRequestLike[] = [];
const shownRequests = new WeakMap<object, { answered: number; requests: readonly PermissionRequestLike[] }>();

function owningSessionId(context: FlowChatContext | null, sessionId: string): string | undefined {
  const session = context?.flowChatStore.getState().sessions.get(sessionId);
  return resolveDelegatedSessionId(sessionId, session);
}

function pendingRequests(sessionId: string): readonly PermissionRequestLike[] {
  const ownerId = resolveDelegatedSessionId(sessionId, undefined) ?? sessionId;
  const entry = delegatedSessionEntry(ownerId);
  if (!entry || entry.pendingPermissions.length === 0) return EMPTY;
  const answered = answeredRequestIds.get(ownerId);
  const cached = shownRequests.get(entry.pendingPermissions);
  if (cached && cached.answered === (answered?.size ?? 0)) return cached.requests;
  // The owner approves each action once: no saved grants on another account's behalf.
  const requests = entry.pendingPermissions
    .filter(request => !answered?.has(String(request.requestId)))
    .map(({ saveResources: _saveResources, ...request }) => request);
  shownRequests.set(entry.pendingPermissions, { answered: answered?.size ?? 0, requests });
  return requests;
}

async function answer(sessionId: string, requestId: string, reply: PermissionReplyKindLike): Promise<void> {
  const ownerId = resolveDelegatedSessionId(sessionId, undefined) ?? sessionId;
  const entry = delegatedSessionEntry(ownerId);
  if (!entry) throw readOnlyError();
  await delegationAPI.answerTask(entry.jobId, requestId, reply === 'reject' ? 'reject' : 'once');
  const answered = answeredRequestIds.get(ownerId) ?? new Set<string>();
  answered.add(requestId);
  answeredRequestIds.set(ownerId, answered);
  // Wake subscribers so the answered request leaves the band at once.
  delegatedSessionStore.getState().update(ownerId, { pendingPermissions: [...entry.pendingPermissions] });
}

function removeLocally(context: FlowChatContext, sessionId: string, removal: SessionCascadeRemoval): void {
  const ownerId = owningSessionId(context, sessionId);
  context.flowChatStore.removeSession(
    sessionId,
    removal.removedActiveSession ? { nextActiveSessionId: null } : undefined,
  );
  removal.removedSessionIds.forEach(id => {
    context.processingManager.clearSessionStatus(id);
    cleanupSaveState(context, id);
    cleanupSessionBuffers(context, id);
  });
  if (ownerId === sessionId) {
    forgetDelegatedTask(sessionId);
    answeredRequestIds.delete(sessionId);
  }
}

export const delegatedSessionDriver: SessionDriver = {
  id: 'delegated',
  // Composer ownership changes when a task's session starts being followed.
  ownershipSource: { subscribe: listener => delegatedSessionStore.subscribe(listener) },

  async createSession(): Promise<string> {
    throw readOnlyError();
  },

  async deleteSession(context, sessionId, removal) {
    // Closing the view here; the task and its session stay with the runner.
    removeLocally(context, sessionId, removal);
  },

  async archiveSession(context, sessionId, removal) {
    removeLocally(context, sessionId, removal);
  },

  async renameSession(context, sessionId) {
    return context.flowChatStore.getState().sessions.get(sessionId)?.title ?? '';
  },

  ensureReady() {},

  async cancel() {
    return false;
  },

  planSubmission() {
    return { kind: 'reject', reason: readOnlyReason() };
  },

  async steer() {
    throw readOnlyError();
  },

  async startTurn() {
    throw readOnlyError();
  },

  async compactSession() {
    throw readOnlyError();
  },

  async runUsageReport() {
    return { shown: false };
  },

  permissionRequestSource(sessionId: string) {
    return {
      subscribe: (listener: () => void) => delegatedSessionStore.subscribe(listener),
      getSnapshot: () => pendingRequests(sessionId),
    };
  },

  async respondPermission(sessionId, requestId, reply) {
    await answer(sessionId, requestId, reply);
  },

  async respondPermissionBatch(sessionId, requestId, reply) {
    const pending = pendingRequests(sessionId) as unknown as PermissionRequest[];
    const batch = selectActivePermissionBatch(pending, sessionId);
    for (const id of batch?.requests.map(request => request.requestId) ?? [requestId]) {
      await answer(sessionId, id, reply);
    }
    return [];
  },
};
