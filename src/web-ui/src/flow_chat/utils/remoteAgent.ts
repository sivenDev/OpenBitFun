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
