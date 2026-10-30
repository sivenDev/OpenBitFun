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

  async closeAgent(id: string): Promise<boolean> {
    try {
      return await api.invoke<boolean>('delegation_close_agent', { request: { id } });
    } catch (error) {
      throw createTauriCommandError('delegation_close_agent', error);
    }
  }
}

export const delegationAPI = new DelegationAPI();
