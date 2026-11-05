import type { SharedWithMe } from '@/infrastructure/api/service-api/LanShareAPI';
import { assistantShareId } from '@/infrastructure/lan-share/lanShareStore';
import type { WorkspaceInfo } from '@/shared/types';
import type { SessionDelegatedBy } from '@/shared/types/session-history';
import type { Session } from '../types/flow-chat';
import { resolveAssistantName } from '@/shared/utils/assistantName';

const SHARED_ASSISTANT_TYPE_PREFIX = 'shared:';
const ASSISTANT_TYPE_PREFIX = 'assistant:';

export interface RemoteAgentAddress {
  deviceId: string;
  assistantId: string;
}

/**
 * Parses `shared:<ownerDeviceId>/<assistantId>`, the type of an assistant
 * another device shares with this one. Assistant ids never contain `/`, so
 * the last separator splits the two parts.
 */
export function remoteAgentFromAgentType(
  agentType: string | null | undefined,
): RemoteAgentAddress | null {
  const value = agentType?.trim();
  if (!value?.startsWith(SHARED_ASSISTANT_TYPE_PREFIX)) return null;
  const rest = value.slice(SHARED_ASSISTANT_TYPE_PREFIX.length);
  const separator = rest.lastIndexOf('/');
  if (separator <= 0 || separator === rest.length - 1) return null;
  return {
    deviceId: rest.slice(0, separator),
    assistantId: rest.slice(separator + 1),
  };
}

/** The id of this device's assistant an `assistant:<id>` type names. */
export function localAssistantFromAgentType(agentType: string | null | undefined): string | null {
  const value = agentType?.trim();
  if (!value?.startsWith(ASSISTANT_TYPE_PREFIX)) return null;
  const id = value.slice(ASSISTANT_TYPE_PREFIX.length);
  return id && !id.includes('/') ? id : null;
}

/**
 * The type a call in a conversation names. In a conversation with an
 * assistant another device shares, the conversation runs on that owner's
 * device, so `assistant:<id>` there is one of the owner's assistants, read
 * as `shared:<owner>/<id>`, and `shared:<this device>/<id>` is one of this
 * device's own, read as `assistant:<id>`.
 */
export function calledTypeInConversation(
  agentType: string | null | undefined,
  conversationOwnerDeviceId: string | null | undefined,
  localDeviceId?: string | null,
): string | undefined {
  const local = localAssistantFromAgentType(agentType);
  if (local && conversationOwnerDeviceId) {
    return `${SHARED_ASSISTANT_TYPE_PREFIX}${conversationOwnerDeviceId}/${local}`;
  }
  const remote = remoteAgentFromAgentType(agentType);
  if (remote && localDeviceId && remote.deviceId === localDeviceId) {
    return `${ASSISTANT_TYPE_PREFIX}${remote.assistantId}`;
  }
  return agentType ?? undefined;
}

/** The owner device of `session` when it is a conversation with a shared assistant. */
export function sharedConversationOwnerDevice(
  session: Pick<Session, 'config'> | null | undefined,
): string | null {
  const target = session?.config?.dispatchTarget;
  return target?.kind === 'sharedAssistant' ? target.ownerDeviceId : null;
}

/** The name this device knows the owner device `deviceId` by, if any. */
function ownerName(deviceId: string, sharedWithMe: readonly SharedWithMe[]): string | null {
  const entry = sharedWithMe.find(candidate => candidate.owner.deviceId === deviceId);
  if (!entry) return null;
  return entry.owner.displayName.trim() || entry.owner.deviceName.trim() || null;
}

/**
 * The name and owner a shared assistant is listed under on this device, or
 * `null` before the list is read or once the assistant is gone from it.
 */
export function sharedAssistantNames(
  address: RemoteAgentAddress,
  sharedWithMe: readonly SharedWithMe[],
): { name: string; owner: string } | null {
  const entry = sharedWithMe.find((candidate) => (
    candidate.owner.deviceId === address.deviceId && candidate.assistantId === address.assistantId
  ));
  const name = entry?.name.trim();
  if (!entry || !name) return null;
  return { name, owner: entry.owner.displayName.trim() || entry.owner.deviceName.trim() };
}

/** The name of this device's assistant in directory `id`, when it is known. */
export function localAssistantName(
  id: string,
  workspaces: readonly WorkspaceInfo[] | null | undefined,
): string | null {
  const workspace = workspaces?.find((candidate) => (
    !candidate.connectionId && assistantShareId(candidate.rootPath) === id
  ));
  return workspace ? resolveAssistantName(workspace) : null;
}

/**
 * The name of the assistant `agentType` calls, one of this device's or one
 * shared with it, or `null` for any other agent or before its list is read.
 */
export function calledAssistantName(
  agentType: string | null | undefined,
  workspaces: readonly WorkspaceInfo[] | null | undefined,
  sharedWithMe: readonly SharedWithMe[],
): string | null {
  const local = localAssistantFromAgentType(agentType);
  if (local) return localAssistantName(local, workspaces);
  const remote = remoteAgentFromAgentType(agentType);
  return remote ? sharedAssistantNames(remote, sharedWithMe)?.name ?? null : null;
}

/**
 * Whose assistant a call reaches: this device's own, or one shared from the
 * named owner's device. Null when the call is not to an assistant.
 */
export function calledAssistantOwner(
  agentType: string | null | undefined,
  session: { delegatedBy?: SessionDelegatedBy } | null | undefined,
  sharedWithMe: readonly SharedWithMe[],
): { kind: 'mine' } | { kind: 'shared'; owner: string } | null {
  const remote = remoteAgentFromAgentType(agentType);
  if (remote) {
    // An owner's assistant it does not share with this device is still
    // named by its owner.
    const owner = sharedAssistantNames(remote, sharedWithMe)?.owner
      || ownerName(remote.deviceId, sharedWithMe);
    return { kind: 'shared', owner: owner || remote.deviceId };
  }
  if (localAssistantFromAgentType(agentType) || session?.delegatedBy) return { kind: 'mine' };
  return null;
}

/**
 * The name of this device's assistant whose session `session` is, when an
 * assistant call started it. Such a session runs as `Claw` in the called
 * assistant's workspace, so its own agent type never names the assistant.
 */
export function calledAssistantSessionName(
  session: { delegatedBy?: SessionDelegatedBy; workspaceId?: string } | null | undefined,
  workspaces: readonly WorkspaceInfo[] | null | undefined,
): string | null {
  if (!session?.delegatedBy || !session.workspaceId) return null;
  const workspace = workspaces?.find((candidate) => (
    !candidate.connectionId && candidate.id === session.workspaceId
  ));
  return workspace ? resolveAssistantName(workspace) : null;
}

/**
 * How a call to an assistant reads to the user: the assistant's name and
 * whose it is, never its internal type. `name` is null for a shared
 * assistant before this device reads its list. Null when the call is not to
 * an assistant.
 */
export function calledAssistantLabels(
  agentType: string | null | undefined,
  session: { delegatedBy?: SessionDelegatedBy; workspaceId?: string } | null | undefined,
  workspaces: readonly WorkspaceInfo[] | null | undefined,
  sharedWithMe: readonly SharedWithMe[],
): { name: string | null; owner: NonNullable<ReturnType<typeof calledAssistantOwner>> } | null {
  const owner = calledAssistantOwner(agentType, session, sharedWithMe);
  if (!owner) return null;
  const name = calledAssistantName(agentType, workspaces, sharedWithMe)
    ?? calledAssistantSessionName(session, workspaces);
  return { name, owner };
}

/**
 * The agent type a tool call in `session` asked for, read from its input. A
 * called assistant's own session runs as `Claw`; the call names the assistant.
 */
export function requestedAgentTypeForToolCall(
  session: Pick<Session, 'dialogTurns'> | null | undefined,
  toolCallId: string | null | undefined,
): string | null {
  if (!toolCallId) return null;
  for (const turn of session?.dialogTurns ?? []) {
    for (const round of turn.modelRounds) {
      for (const item of round.items) {
        if (item.type !== 'tool' || (item.toolCall?.id ?? item.id) !== toolCallId) continue;
        const input = item.toolCall?.input as Record<string, unknown> | undefined;
        const type = input?.agent_type ?? input?.agentType;
        return typeof type === 'string' && type.trim() ? type.trim() : null;
      }
    }
  }
  return null;
}

/**
 * The model a call to a shared assistant ran on, as its owner's device
 * reports it. Earlier builds named those rounds after the device, which is
 * not a model.
 */
export function reportedSharedModel(
  agentType: string | null | undefined,
  ...reported: Array<string | null | undefined>
): string | null {
  const remote = remoteAgentFromAgentType(agentType);
  if (!remote) return null;
  for (const value of reported) {
    const name = value?.trim();
    if (name && name !== remote.deviceId) return name;
  }
  return null;
}

/**
 * The model a called agent's session last ran on, from its newest model round
 * that recorded one, or null before it ran.
 */
export function latestEffectiveModel(session: Pick<Session, 'dialogTurns'> | null | undefined): string | null {
  const turns = session?.dialogTurns ?? [];
  for (let turnIndex = turns.length - 1; turnIndex >= 0; turnIndex--) {
    const rounds = turns[turnIndex].modelRounds ?? [];
    for (let roundIndex = rounds.length - 1; roundIndex >= 0; roundIndex--) {
      const name = rounds[roundIndex].effectiveModelName?.trim();
      if (name) return name;
    }
  }
  return null;
}
