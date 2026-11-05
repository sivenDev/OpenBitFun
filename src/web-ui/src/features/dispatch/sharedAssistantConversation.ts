/**
 * Conversations with assistants other devices share on the local network.
 *
 * Each conversation is a Dispatch job whose target is the shared assistant.
 * It belongs to that assistant, not to a workspace of this device (README
 * invariant 33), so it is found by owner device and assistant id.
 */

import { flowChatManager } from '@/flow_chat/services/FlowChatManager';
import { openMainSession } from '@/flow_chat/services/sessionActivation';
import { flowChatStore } from '@/flow_chat/store/FlowChatStore';
import { lanShareAPI } from '@/infrastructure/api/service-api/LanShareAPI';
import { dispatchJobStore, type DispatchObserverJob } from './dispatchJobStore';

/** One shared assistant, as `SharedWithMe` names it. */
export interface SharedAssistantRef {
  ownerDeviceId: string;
  assistantId: string;
  name: string;
}

function isConversationWith(job: DispatchObserverJob, assistant: SharedAssistantRef): boolean {
  return job.target.kind === 'sharedAssistant'
    && job.target.ownerDeviceId === assistant.ownerDeviceId
    && job.target.assistantId === assistant.assistantId;
}

/** The conversations with `assistant`, most recent first. */
export function sharedAssistantConversations(
  jobs: Record<string, DispatchObserverJob>,
  assistant: SharedAssistantRef,
): DispatchObserverJob[] {
  return Object.values(jobs)
    .filter(job => isConversationWith(job, assistant))
    .sort((left, right) => right.updatedAt - left.updatedAt);
}

/** Starts a new conversation with `assistant` and shows it. */
export async function startSharedAssistantConversation(assistant: SharedAssistantRef): Promise<string> {
  const sessionId = await flowChatManager.createSharedAssistantSession({
    ownerDeviceId: assistant.ownerDeviceId,
    assistantId: assistant.assistantId,
    displayName: assistant.name,
  });
  await openMainSession(sessionId);
  return sessionId;
}

/**
 * Whether `job` is a new conversation whose first message never left this
 * device and whose projection did not survive a restart: nothing records it
 * but this store, so there is nothing to open.
 */
export function isUnsentConversation(job: DispatchObserverJob): boolean {
  return job.state === 'submitting' && !flowChatStore.getState().sessions.has(job.sessionId);
}

/**
 * Shows the conversation `job`. A new conversation that was never sent and
 * no longer has a projection is dropped and replaced by a new one.
 */
export async function openSharedAssistantConversationJob(
  job: DispatchObserverJob,
  assistant: SharedAssistantRef,
): Promise<string> {
  if (isUnsentConversation(job)) {
    dispatchJobStore.getState().removeJob(job.jobId);
    return startSharedAssistantConversation(assistant);
  }
  await openMainSession(job.sessionId);
  return job.sessionId;
}

/**
 * Shows the latest conversation with `assistant`, or starts one when there
 * is none and `canStart` allows it. Returns the session shown, if any.
 */
export async function openSharedAssistantConversation(
  assistant: SharedAssistantRef,
  canStart: boolean,
): Promise<string | undefined> {
  const [latest] = sharedAssistantConversations(dispatchJobStore.getState().jobs, assistant);
  if (latest && (canStart || !isUnsentConversation(latest))) {
    return openSharedAssistantConversationJob(latest, assistant);
  }
  if (!canStart) return undefined;
  return startSharedAssistantConversation(assistant);
}

/**
 * Forgets the conversations with `assistant` on this device: each projection,
 * its outbound record and its cached transcript, as deleting a conversation
 * does. The owner keeps its own copy until its retention ends.
 */
export async function forgetSharedAssistantConversations(assistant: SharedAssistantRef): Promise<void> {
  const conversations = sharedAssistantConversations(dispatchJobStore.getState().jobs, assistant);
  for (const job of conversations) {
    await flowChatManager.deleteChatSession(job.sessionId);
  }
}

/**
 * Whether `job` never reached the owner: a new conversation whose first
 * message was not sent. Only this device knows of it.
 */
export function isLocalOnlyConversation(job: DispatchObserverJob): boolean {
  return job.state === 'submitting'
    && !flowChatStore.getState().sessions.get(job.sessionId)?.dialogTurns?.length;
}

/** Owners that answered whether they delete conversations, for this run. */
const deleteSupport = new Map<string, Promise<boolean>>();

/**
 * Whether the owner device deletes this device's conversations with its
 * assistants. Asks the owner once per run; an owner that cannot be asked is
 * asked again next time.
 */
export function sharedConversationDeleteSupported(ownerDeviceId: string): Promise<boolean> {
  let support = deleteSupport.get(ownerDeviceId);
  if (!support) {
    support = lanShareAPI.conversationSupport(ownerDeviceId).then(answer => answer.deleteConversation === true);
    support.catch(() => deleteSupport.delete(ownerDeviceId));
    deleteSupport.set(ownerDeviceId, support);
  }
  return support;
}

/** Forgets what owners answered; for tests. */
export function resetSharedConversationSupportForTest(): void {
  deleteSupport.clear();
}

/**
 * Deletes the conversation `job`: on the owner's device first, unless it
 * never reached it, then here.
 */
export async function deleteSharedAssistantConversation(job: DispatchObserverJob): Promise<void> {
  if (!isLocalOnlyConversation(job)) {
    await lanShareAPI.deleteConversation(job.jobId);
  }
  if (flowChatStore.getState().sessions.has(job.sessionId)) {
    await flowChatManager.deleteChatSession(job.sessionId);
  } else {
    dispatchJobStore.getState().dismissJob(job.jobId);
  }
}
