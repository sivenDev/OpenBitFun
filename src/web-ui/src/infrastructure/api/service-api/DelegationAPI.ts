import { api } from './ApiClient';
import { createTauriCommandError } from '../errors/TauriCommandError';

/** Task kinds a delegated job may run. Unknown kinds from newer builds are kept as strings. */
export type DelegationTaskKind = 'message' | 'coding' | (string & {});

/** Grants every device of this account. */
export const ALL_SAME_ACCOUNT_DEVICES = '*';

export interface OpenAgent {
  id: string;
  agentType: string;
  displayName: string;
  description?: string;
  taskKinds: DelegationTaskKind[];
  messageWorkspace?: string;
  sameAccountDevices: string[];
  /** Cross-account connections granted one by one; absent when none. */
  connections?: ConnectionGrant[];
}

/** One cross-account connection an open agent is granted to. */
export interface ConnectionGrant {
  connectionId: string;
}

/** One stored entry; `agent` is absent when this build cannot read it. */
export interface OpenAgentRecord {
  agent?: OpenAgent;
  problem?: string;
  raw?: Record<string, unknown>;
}

export interface OpenAgentRequest {
  id: string;
  agentType: string;
  displayName: string;
  description?: string | null;
  taskKinds: DelegationTaskKind[];
  messageWorkspace?: string | null;
  sameAccountDevices: string[];
  /** The owner confirmed an external agent's usage notice; required for `acp:` agents. */
  usageConfirmed?: boolean;
}

/** The ACP client an external (`acp:<id>`) agent type names, if any. */
export function externalAgentClientId(agentType: string): string | null {
  const clientId = agentType.startsWith('acp:') ? agentType.slice(4).trim() : '';
  return clientId === '' ? null : clientId;
}

export function openAgentRecordId(record: OpenAgentRecord): string | null {
  if (record.agent) return record.agent.id;
  const id = record.raw?.id;
  return typeof id === 'string' ? id : null;
}

/** Device-local list of agents opened to other devices of this account. */
class DelegationAPI {
  async listOpenAgents(): Promise<OpenAgentRecord[]> {
    try {
      return await api.invoke<OpenAgentRecord[]>('delegation_list_open_agents');
    } catch (error) {
      throw createTauriCommandError('delegation_list_open_agents', error);
    }
  }

  async openAgent(request: OpenAgentRequest): Promise<OpenAgent> {
    try {
      return await api.invoke<OpenAgent>('delegation_open_agent', { request });
    } catch (error) {
      throw createTauriCommandError('delegation_open_agent', error);
    }
  }

  /** Grants an open agent to a paired device of another account. */
  async grantConnection(id: string, connectionId: string, usageConfirmed: boolean): Promise<OpenAgent> {
    const request = { id, connectionId, usageConfirmed };
    try {
      return await api.invoke<OpenAgent>('delegation_grant_connection', { request });
    } catch (error) {
      throw createTauriCommandError('delegation_grant_connection', error, request);
    }
  }

  /** Withdraws an open agent from a connection. */
  async revokeConnection(id: string, connectionId: string): Promise<boolean> {
    const request = { id, connectionId };
    try {
      return await api.invoke<boolean>('delegation_revoke_connection', { request });
    } catch (error) {
      throw createTauriCommandError('delegation_revoke_connection', error, request);
    }
  }

  async closeAgent(id: string): Promise<boolean> {
    try {
      return await api.invoke<boolean>('delegation_close_agent', { request: { id } });
    } catch (error) {
      throw createTauriCommandError('delegation_close_agent', error);
    }
  }
}

export const delegationAPI = new DelegationAPI();
