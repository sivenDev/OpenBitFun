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

/** Most this device runs for one connection with one open agent; absent fields mean no limit. */
export interface ConnectionLimits {
  /** Tasks that may run at once (1-100). */
  maxConcurrentJobs?: number;
  /** Minutes one turn may run before it is stopped (1-1440). */
  maxJobMinutes?: number;
}

export const MAX_CONNECTION_CONCURRENT_JOBS = 100;
export const MAX_CONNECTION_JOB_MINUTES = 24 * 60;

/** One cross-account connection an open agent is granted to. */
export interface ConnectionGrant extends ConnectionLimits {
  connectionId: string;
}

/** A cross-account task event in this device's audit log. */
export interface DelegationAuditEntry {
  at: string;
  /** `submitted`, `finished`, or `unknown` for an event from a newer build. */
  event: 'submitted' | 'finished' | 'unknown' | (string & {});
  jobId: string;
  connectionId: string;
  peerUserId: string;
  peerDisplayName: string;
  peerDeviceId: string;
  peerDeviceName: string;
  openAgentId: string;
  taskKind: DelegationTaskKind;
  /** The job state a finished entry ended in. */
  state?: string;
  detail?: string;
}

/** One line of the audit log; `entry` is absent when the line cannot be read. */
export interface DelegationAuditRecord {
  entry?: DelegationAuditEntry;
  problem?: string;
  raw?: string;
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

  /** Sets the limits of a grant; absent fields remove that limit. */
  async setConnectionLimits(id: string, connectionId: string, limits: ConnectionLimits): Promise<OpenAgent> {
    const request = { id, connectionId, ...limits };
    try {
      return await api.invoke<OpenAgent>('delegation_set_connection_limits', { request });
    } catch (error) {
      throw createTauriCommandError('delegation_set_connection_limits', error, request);
    }
  }

  /** Tasks other accounts delegated to this device, oldest first. */
  async listAudit(limit?: number): Promise<DelegationAuditRecord[]> {
    const request = { limit: limit ?? null };
    try {
      return await api.invoke<DelegationAuditRecord[]>('delegation_audit_list', { request });
    } catch (error) {
      throw createTauriCommandError('delegation_audit_list', error, request);
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
