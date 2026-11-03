/**
 * Conversations with assistants other devices share on the local network.
 *
 * Each conversation is a Dispatch job whose target is the shared assistant.
 * It belongs to that assistant, not to a workspace of this device (README
 * invariant 33), so it is found by owner device and assistant id.
 */

import { flowChatManager } from '@/flow_chat/services/FlowChatManager';
import { openMainSession } from '@/flow_chat/services/sessionActivation';
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
 * Shows the latest conversation with `assistant`, or starts one when there
 * is none and `canStart` allows it. Returns the session shown, if any.
 */
export async function openSharedAssistantConversation(
  assistant: SharedAssistantRef,
  canStart: boolean,
): Promise<string | undefined> {
  const [latest] = sharedAssistantConversations(dispatchJobStore.getState().jobs, assistant);
  if (latest) {
    await openMainSession(latest.sessionId);
    return latest.sessionId;
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
