import type { SharedWithMe } from '@/infrastructure/api/service-api/LanShareAPI';
import { assistantShareId } from '@/infrastructure/lan-share/lanShareStore';
import type { WorkspaceInfo } from '@/shared/types';
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
