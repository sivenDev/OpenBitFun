import { invokePrepared } from './invokePrepared';
/**
 * Subagent API
 */

import { api } from './ApiClient';
import { workspaceScopedRequest } from './legacyWorkspaceCompatibility';



export type SubagentSource = 'builtin' | 'project' | 'user' | 'external';
export type BuiltinSubagentExposure = 'public' | 'restricted' | 'hidden';
export type SubagentOverrideState = 'enabled' | 'disabled';
export type SubagentStateReason =
  | 'builtin_default_visible'
  | 'builtin_default_hidden'
  | 'custom_default_enabled'
  | 'enabled_by_project_override'
  | 'disabled_by_project_override'
  | 'enabled_by_user_override'
  | 'disabled_by_user_override';

export interface SubagentVisibilitySummary {
  exposure: BuiltinSubagentExposure;
  allowedParentAgentIds: string[];
  deniedParentAgentIds: string[];
  showInGlobalRegistry: boolean;
}

export type ExternalAgentUnavailableReason =
  | 'executor_missing'
  | 'not_enabled'
  | 'delegation_disabled'
  | 'not_installed'
  | 'unsupported_context';

/** Host-level availability of an external agent, as reported by its executor. */
export type ExternalAgentAvailability =
  | { state: 'available' }
  | { state: 'unavailable'; reason: ExternalAgentUnavailableReason | string; detail: string };

export type RemoteAgentUnavailableReason = 'offline' | 'incompatible' | 'unreachable' | 'agent_unavailable';

/** Whether the device that opened a remote agent can take work now. */
export type RemoteAgentAvailability =
  | { state: 'available' }
  | { state: 'unavailable'; reason: RemoteAgentUnavailableReason | string; detail: string };

/** The device and open agent behind a `remote:<deviceId>/<openAgentId>` subagent. */
export interface RemoteAgentInfo {
  deviceId: string;
  deviceName: string;
  openAgentId: string;
  /** The assistant the open agent runs as on its device (`assistant:<directory>`). */
  agentType: string;
  availability: RemoteAgentAvailability;
  /** The other account, for an agent reached over a cross-account connection. */
  account?: { userId: string; displayName: string };
}

export interface SubagentInfo {
  key: string;
  id: string;
  name: string;
  description: string;
  isReadonly: boolean;
  isReview: boolean;
  toolCount: number;
  defaultTools: string[];
  defaultEnabled: boolean;
  effectiveEnabled: boolean;
  overrideState?: SubagentOverrideState;
  stateReason?: SubagentStateReason;
  source?: SubagentSource;
  subagentSource?: SubagentSource;
  path?: string;
  model?: string;
  modelIsExplicit?: boolean;
  visibility?: SubagentVisibilitySummary;
  configProfileId?: string;
  configProfileLabel?: string;
  configProfileMemberModeIds?: string[];
  externalProviderLabel?: string;
  supportsFollowUp?: boolean;
  /**
   * `acp` for external agents that run their own model loop and tools;
   * `remote` for agents another device of this account opened to this one.
   */
  executionKind?: string;
  externalAvailability?: ExternalAgentAvailability;
  remoteAgent?: RemoteAgentInfo;
}

export interface ListSubagentsOptions {
  source?: SubagentSource;
  workspaceId?: string;
}

export interface ListVisibleSubagentsOptions {
  workspaceId?: string;
  parentAgentType: string;
}

export interface ListManageableSubagentsOptions {
  workspaceId?: string;
  parentAgentType: string;
}

export interface ReloadSubagentsOptions {
  workspaceId?: string;
}

export type SubagentLevel = 'user' | 'project';

export interface CreateSubagentPayload {
  level: SubagentLevel;
  name: string;
  description: string;
  prompt: string;
  tools?: string[];
   
  readonly?: boolean;
  review?: boolean;
  workspaceId?: string;
}

export interface UpdateSubagentConfigPayload {
  subagentId: string;
  parentAgentType?: string;
  enabled?: boolean;
  model?: string;
  clearModelOverride?: boolean;
  workspaceId?: string;
}

export interface UpdateSubagentConfigResponse {
  availabilityUpdated: boolean;
  modelUpdated: boolean;
}

/** Full definition for create/edit form (custom user/project sub-agents) */
export interface SubagentDetail {
  subagentId: string;
  name: string;
  description: string;
  prompt: string;
  tools: string[];
  readonly: boolean;
  review: boolean;
  enabled: boolean;
  model: string;
  path: string;
  level: SubagentLevel;
}

export interface GetSubagentDetailPayload {
  subagentId: string;
  workspaceId?: string;
}

export interface UpdateSubagentPayload {
  subagentId: string;
  description: string;
  prompt: string;
  tools?: string[];
  readonly?: boolean;
  review?: boolean;
  workspaceId?: string;
}

// ==================== API ====================

export const SubagentAPI = {
   
  async listSubagents(options?: ListSubagentsOptions): Promise<SubagentInfo[]> {
    return invokePrepared<SubagentInfo[]>('list_subagents', async () => ({
      request: await workspaceScopedRequest(options ?? {}),
    }));
  },

  async listVisibleSubagents(options: ListVisibleSubagentsOptions): Promise<SubagentInfo[]> {
    return invokePrepared<SubagentInfo[]>('list_visible_subagents', async () => ({
      request: await workspaceScopedRequest(options),
    }));
  },

  async listManageableSubagents(options: ListManageableSubagentsOptions): Promise<SubagentInfo[]> {
    return invokePrepared<SubagentInfo[]>('list_manageable_subagents', async () => ({
      request: await workspaceScopedRequest(options),
    }));
  },

   
  async reloadSubagents(options: ReloadSubagentsOptions = {}): Promise<void> {
    return invokePrepared('reload_subagents', async () => ({
      request: await workspaceScopedRequest(options),
    }));
  },

   
  async createSubagent(payload: CreateSubagentPayload): Promise<void> {
    return invokePrepared('create_subagent', async () => ({
      request: await workspaceScopedRequest(payload),
    }));
  },

   
  async listAgentToolNames(): Promise<string[]> {
    return api.invoke<string[]>('list_agent_tool_names');
  },

   
  async updateSubagentConfig(
    payload: UpdateSubagentConfigPayload,
  ): Promise<UpdateSubagentConfigResponse> {
    return invokePrepared<UpdateSubagentConfigResponse>('update_subagent_config', async () => ({
      request: await workspaceScopedRequest(payload),
    }));
  },

  async getSubagentDetail(payload: GetSubagentDetailPayload): Promise<SubagentDetail> {
    const raw = await invokePrepared<SubagentDetail & { level: string }>('get_subagent_detail', async () => ({
      request: await workspaceScopedRequest({
        subagentId: payload.subagentId,
        workspaceId: payload.workspaceId,
      }),
    }));
    return {
      ...raw,
      level: raw.level === 'project' ? 'project' : 'user',
    };
  },

  async updateSubagent(payload: UpdateSubagentPayload): Promise<void> {
    return invokePrepared('update_subagent', async () => ({
      request: await workspaceScopedRequest({
        subagentId: payload.subagentId,
        description: payload.description,
        prompt: payload.prompt,
        tools: payload.tools,
        readonly: payload.readonly,
        review: payload.review,
        workspaceId: payload.workspaceId,
      }),
    }));
  },

  async deleteSubagent(subagentId: string, workspaceId?: string): Promise<void> {
    return invokePrepared('delete_subagent', async () => ({
      request: await workspaceScopedRequest({ subagentId, workspaceId }),
    }));
  },
};
