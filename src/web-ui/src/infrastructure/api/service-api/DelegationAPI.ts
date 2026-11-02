import { api } from './ApiClient';
import type { DispatchStatusResponse } from '@/features/dispatch/types';
import { createTauriCommandError } from '../errors/TauriCommandError';

/** The `assistant:` prefix of the agent type that opens an assistant. */
export const ASSISTANT_AGENT_TYPE_PREFIX = 'assistant:';

/** The device a task came from, as its grant named it. */
export interface DelegatedTaskSource {
  deviceId: string;
  /** The fingerprint the owner confirmed when granting the device. */
  fingerprint: string;
  displayName: string;
  deviceName: string;
}

/** A task another device gave a shared assistant here. */
export interface DelegatedTask {
  jobId: string;
  sessionId: string;
  state: string;
  startedAt?: string;
  title: string;
  /** The shared assistant: its directory. */
  assistantId: string;
  /** The assistant the task runs as. */
  agentType: string;
  assistantWorkspace?: string;
  pendingPermissionCount: number;
  source: DelegatedTaskSource;
}

/** The owner's view of tasks other devices gave this device's shared assistants. */
class DelegationAPI {
  /** Tasks other devices gave this device's shared assistants, newest first. */
  async listTasks(): Promise<DelegatedTask[]> {
    try {
      return await api.invoke<DelegatedTask[]>('delegation_list_tasks');
    } catch (error) {
      throw createTauriCommandError('delegation_list_tasks', error);
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
}

export const delegationAPI = new DelegationAPI();
