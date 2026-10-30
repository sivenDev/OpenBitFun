const REMOTE_AGENT_TYPE_PREFIX = 'remote:';

export interface RemoteAgentAddress {
  deviceId: string;
  openAgentId: string;
}

/**
 * Parses `remote:<deviceId>/<openAgentId>`, the type of an agent another
 * device of this account opened to this one. Open agent ids never contain
 * `/`, so the last separator splits the two parts.
 */
export function remoteAgentFromAgentType(
  agentType: string | null | undefined,
): RemoteAgentAddress | null {
  const value = agentType?.trim();
  if (!value?.startsWith(REMOTE_AGENT_TYPE_PREFIX)) return null;
  const rest = value.slice(REMOTE_AGENT_TYPE_PREFIX.length);
  const separator = rest.lastIndexOf('/');
  if (separator <= 0 || separator === rest.length - 1) return null;
  return {
    deviceId: rest.slice(0, separator),
    openAgentId: rest.slice(separator + 1),
  };
}
