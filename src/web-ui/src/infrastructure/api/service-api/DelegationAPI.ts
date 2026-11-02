import { api } from './ApiClient';
import type { DispatchStatusResponse } from '@/features/dispatch/types';
import { createTauriCommandError } from '../errors/TauriCommandError';

/** Grants every device of this account. */
export const ALL_SAME_ACCOUNT_DEVICES = '*';

export interface OpenAgent {
  id: string;
  agentType: string;
  displayName: string;
  description?: string;
  sameAccountDevices: string[];
  /** Days the tasks of same-account devices are kept after their last turn; absent means 7. */
  sessionRetentionDays?: number;
  /** Cross-account connections granted one by one; absent when none. */
  connections?: ConnectionGrant[];
}

/** Most this device runs for one connection with one open agent; absent fields mean no limit. */
export interface ConnectionLimits {
  /** Tasks that may run at once (1-100). */
  maxConcurrentJobs?: number;
  /** Minutes one turn may run before it is stopped (1-1440). */
  maxJobMinutes?: number;
  /** Days the connection's tasks and their sessions are kept after their last turn; absent means 7. */
  sessionRetentionDays?: number;
}

export const MAX_CONNECTION_CONCURRENT_JOBS = 100;
export const MAX_CONNECTION_JOB_MINUTES = 24 * 60;
export const DEFAULT_SESSION_RETENTION_DAYS = 7;
/** The retention periods an owner can choose, in days. */
export const SESSION_RETENTION_DAY_CHOICES = [1, 7, 30, 90] as const;

/** One cross-account connection an open agent is granted to. */
export interface ConnectionGrant extends ConnectionLimits {
  connectionId: string;
}

/** A cross-account task event in this device's audit log. */
export interface DelegationAuditEntry {
  at: string;
  event: 'submitted' | 'finished';
  jobId: string;
  connectionId: string;
  peerUserId: string;
  peerDisplayName: string;
  peerDeviceId: string;
  peerDeviceName: string;
  openAgentId: string;
  /** How far this device trusted the connection. */
  trust: DelegationTrust;
  /** External agents (`acp:<id>`) the task started. */
  externalAgents?: string[];
  /** The job state a finished entry ended in. */
  state?: string;
  detail?: string;
}

/** How far tasks from a connection are trusted. */
export type DelegationTrust = 'normal' | 'full';

/** The `assistant:` prefix of the agent type that opens an assistant. */
export const ASSISTANT_AGENT_TYPE_PREFIX = 'assistant:';

/** An assistant on this device that can be opened. */
export interface LocalAssistant {
  /** `assistant:<directory>`. */
  agentType: string;
  /** The assistant id, or null for the default assistant. */
  assistantId: string | null;
  rootPath: string;
}

/** Who delegated a task to an assistant here. */
export interface DelegatedTaskSource {
  /** `device` for a device of this account, `connection` for another account. */
  kind: 'device' | 'connection';
  deviceId?: string;
  deviceName?: string;
  connectionId?: string;
  accountName?: string;
  accountUserId?: string;
}

/** A task another device or account delegated to an assistant here. */
export interface DelegatedTask {
  jobId: string;
  sessionId: string;
  state: string;
  startedAt?: string;
  title: string;
  openAgentId: string;
  /** The assistant the task runs as. */
  agentType: string;
  assistantWorkspace?: string;
  pendingPermissionCount: number;
  source: DelegatedTaskSource;
}

/** A prompt a delegated task waits on, as the owner sees it. */
export interface DelegatedTaskPrompt {
  requestId: string;
  action: string;
  resources: string[];
}

/** The state of a delegated task and the prompts it waits on. */
export interface DelegatedTaskStatus {
  state: string;
  cursor: number;
  pendingPermissions: DelegatedTaskPrompt[];
  lastError?: string;
}

/** One line of the audit log; `entry` is absent when the line cannot be read. */
export interface DelegationAuditRecord {
  entry?: DelegationAuditEntry;
  problem?: string;
  raw?: string;
}

/** Opens an assistant (`assistant:<directory>`) to other devices. */
export interface OpenAgentRequest {
  id: string;
  agentType: string;
  displayName: string;
  description?: string | null;
  sameAccountDevices: string[];
  /** The owner confirmed that the assistant's tasks may use external agents with this device's login. */
  usageConfirmed: boolean;
}

/** Device-local list of assistants opened to other devices and accounts. */
class DelegationAPI {
  async listOpenAgents(): Promise<OpenAgent[]> {
    try {
      return await api.invoke<OpenAgent[]>('delegation_list_open_agents');
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

  /** Grants an opened assistant to a paired device of another account. */
  async grantConnection(id: string, connectionId: string): Promise<OpenAgent> {
    const request = { id, connectionId };
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

  /** The assistants on this device. */
  async listAssistants(): Promise<LocalAssistant[]> {
    try {
      return await api.invoke<LocalAssistant[]>('delegation_list_assistants');
    } catch (error) {
      throw createTauriCommandError('delegation_list_assistants', error);
    }
  }

  /** Tasks other devices and accounts delegated to this device's assistants, newest first. */
  async listTasks(): Promise<DelegatedTask[]> {
    try {
      return await api.invoke<DelegatedTask[]>('delegation_list_tasks');
    } catch (error) {
      throw createTauriCommandError('delegation_list_tasks', error);
    }
  }

  async taskStatus(jobId: string): Promise<DelegatedTaskStatus> {
    const request = { jobId, cursor: 0 };
    try {
      return await api.invoke<DelegatedTaskStatus>('delegation_task_status', { request });
    } catch (error) {
      throw createTauriCommandError('delegation_task_status', error, request);
    }
  }

  /** A delegated task's events after `cursor`, in the Dispatch status shape, and the prompts it waits on. */
  async taskEvents(jobId: string, cursor: number): Promise<DispatchStatusResponse> {
    const request = { jobId, cursor };
    try {
      return await api.invoke<DispatchStatusResponse>('delegation_task_status', { request });
    } catch (error) {
      throw createTauriCommandError('delegation_task_status', error, request);
    }
  }

  /** Approves one action of a delegated task, or rejects it. */
  async answerTask(jobId: string, requestId: string, reply: 'once' | 'reject'): Promise<void> {
    const request = { jobId, requestId, reply };
    try {
      await api.invoke('delegation_task_answer', { request });
    } catch (error) {
      throw createTauriCommandError('delegation_task_answer', error, request);
    }
  }

  /** Trust levels of connections trusted fully; others are normal. */
  async listConnectionTrust(): Promise<Record<string, DelegationTrust>> {
    try {
      return await api.invoke<Record<string, DelegationTrust>>('delegation_list_connection_trust');
    } catch (error) {
      throw createTauriCommandError('delegation_list_connection_trust', error);
    }
  }

  /** Sets a connection's trust; full requires the owner to have confirmed the risk. */
  async setConnectionTrust(connectionId: string, level: 'normal' | 'full', riskConfirmed: boolean): Promise<void> {
    const request = { connectionId, level, riskConfirmed };
    try {
      await api.invoke('delegation_set_connection_trust', { request });
    } catch (error) {
      throw createTauriCommandError('delegation_set_connection_trust', error, request);
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
