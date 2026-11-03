import { api } from './ApiClient';
import { createTauriCommandError } from '../errors/TauriCommandError';

/** How much a shared assistant may do for other devices. */
export type ShareTrust = 'normal' | 'full';

/** What the owner chooses when sharing an assistant. */
export interface ShareSettings {
  trust: ShareTrust;
  /** One of 1, 7, 30 or 90. */
  sessionRetentionDays: number;
  /** Tasks that run at once for all users together. */
  maxConcurrentTasks: number;
}

/** What the owner explicitly confirmed with a share or a settings change. */
export interface ShareConfirmations {
  fullTrustRisk?: boolean;
  externalAgentUsage?: boolean;
}

/** A device on the other side of a share. */
export interface SharePeer {
  publicKey: string;
  deviceId: string;
  /** The short code people compare. */
  fingerprint: string;
  displayName: string;
  deviceName: string;
}

export interface ShareGrant extends SharePeer {
  grantedAt: string;
  lastUsedAt?: string;
}

export interface BlockedDevice extends SharePeer {
  blockedAt: string;
}

/** One assistant this device shares, as its owner manages it. */
export interface OwnedShare extends ShareSettings {
  /** The assistant's directory. */
  id: string;
  /** The name other devices see. */
  displayName?: string;
  /** What other devices see when they look the assistant up. */
  description?: string;
  externalAgentUsageConfirmed: boolean;
  sharedAt: string;
  grants: ShareGrant[];
  blocked: BlockedDevice[];
  /** The assistant's workspace, to match it with the assistant list. */
  rootPath: string;
  /** Devices whose data under this assistant is still kept. */
  tenantDeviceIds: string[];
}

/** This device as other devices see it. */
export interface LanShareHostStatus {
  deviceId: string;
  deviceName: string;
  displayName: string;
  fingerprint: string;
  /** Set while this device shares an assistant and listens. */
  port?: number;
  /** Why other devices cannot see or reach this device. */
  problem?: string;
}

export interface LanShareOverview {
  status: LanShareHostStatus;
  shares: OwnedShare[];
  /** Why the shares cannot be read. */
  sharesError?: string;
}

/** A request waiting for the owner's answer. */
export interface ShareRequest {
  requestId: string;
  assistantId: string;
  assistantName: string;
  peer: SharePeer;
  openedAtMs: number;
  deadlineMs: number;
}

export type ShareAnswer = 'allow' | 'decline' | 'block';

/** Where this device stands with a discovered assistant. */
export type ShareAccess =
  | { state: 'available' }
  | { state: 'pending'; requestId: string; deadlineMs: number }
  | { state: 'granted' }
  | { state: 'cooling'; retryAtMs: number };

export interface DiscoveredAssistant {
  id: string;
  name: string;
  description?: string;
  access: ShareAccess;
}

export type DiscoveredDeviceStatus =
  | { state: 'connecting' }
  | { state: 'ready' }
  | { state: 'needsUpdate'; message: string }
  | { state: 'unreachable'; message: string };

/** A sharing device on the local network. */
export interface DiscoveredDevice {
  deviceId: string;
  deviceName: string;
  displayName: string;
  fingerprint: string;
  publicKey: string;
  status: DiscoveredDeviceStatus;
  assistants: DiscoveredAssistant[];
}

/** The answer to a request: open until `deadlineMs`, or already decided. */
export interface ShareRequestOpened {
  requestId: string;
  deadlineMs: number;
  granted?: boolean;
}

/** An assistant another device shared with this one. */
export interface SharedWithMe {
  owner: SharePeer;
  assistantId: string;
  name: string;
  description?: string;
  grantedAt: string;
  state: 'available' | 'revoked';
  lastAddress?: string;
}

export const LAN_SHARE_EVENTS = {
  requests: 'lan_share://requests',
  shared: 'lan_share://shared',
  sharedWithMe: 'lan_share://shared_with_me',
  discovered: 'lan_share://discovered',
} as const;

async function invoke<T>(command: string, request?: Record<string, unknown>): Promise<T> {
  try {
    return await api.invoke<T>(command, request ? { request } : undefined);
  } catch (error) {
    throw createTauriCommandError(command, error, request);
  }
}

/** Sharing assistants with devices on the local network. */
class LanShareAPI {
  // ---- Owner ----

  overview(): Promise<LanShareOverview> {
    return invoke('lan_share_overview');
  }

  share(
    assistantId: string,
    name: string,
    settings: ShareSettings,
    confirmations: ShareConfirmations = {},
    description?: string,
  ): Promise<OwnedShare> {
    return invoke('lan_share_share', { assistantId, name, description, settings, confirmations });
  }

  /**
   * Changes a share's settings or what other devices see of it: its name and
   * description. An empty description clears it.
   */
  update(
    assistantId: string,
    changes: {
      name?: string;
      description?: string;
      settings?: ShareSettings;
      confirmations?: ShareConfirmations;
    },
  ): Promise<OwnedShare> {
    return invoke('lan_share_update', {
      assistantId,
      name: changes.name,
      description: changes.description,
      settings: changes.settings,
      confirmations: changes.confirmations ?? {},
    });
  }

  stop(assistantId: string): Promise<boolean> {
    return invoke('lan_share_stop', { assistantId });
  }

  revoke(assistantId: string, publicKey: string, block = false): Promise<void> {
    return invoke('lan_share_revoke', { assistantId, publicKey, block });
  }

  /** Deletes what a device without access left under a shared assistant. */
  deleteTenant(assistantId: string, deviceId: string): Promise<boolean> {
    return invoke('lan_share_delete_tenant', { assistantId, deviceId });
  }

  requests(): Promise<ShareRequest[]> {
    return invoke('lan_share_requests');
  }

  decide(requestId: string, answer: ShareAnswer): Promise<void> {
    return invoke('lan_share_decide', { requestId, answer });
  }

  setDisplayName(name: string): Promise<LanShareHostStatus> {
    return invoke('lan_share_set_display_name', { name });
  }

  // ---- User ----

  discoverStart(): Promise<DiscoveredDevice[]> {
    return invoke('lan_share_discover_start');
  }

  discoverStop(): Promise<void> {
    return invoke('lan_share_discover_stop');
  }

  discovered(): Promise<DiscoveredDevice[]> {
    return invoke('lan_share_discover_list');
  }

  request(deviceId: string, assistantId: string): Promise<ShareRequestOpened> {
    return invoke('lan_share_request', { deviceId, assistantId });
  }

  sharedWithMe(): Promise<SharedWithMe[]> {
    return invoke('lan_share_shared_with_me');
  }

  remove(ownerKey: string, assistantId: string): Promise<void> {
    return invoke('lan_share_remove', { ownerKey, assistantId });
  }

  /** Opens the system settings that let this app use the local network. */
  openNetworkSettings(): Promise<void> {
    return invoke('lan_share_open_network_settings');
  }

  // ---- Events ----

  onRequests(handler: (payload: { requests: ShareRequest[] }) => void): () => void {
    return api.listen(LAN_SHARE_EVENTS.requests, handler);
  }

  onShared(handler: () => void): () => void {
    return api.listen(LAN_SHARE_EVENTS.shared, handler);
  }

  onSharedWithMe(handler: () => void): () => void {
    return api.listen(LAN_SHARE_EVENTS.sharedWithMe, handler);
  }

  onDiscovered(handler: () => void): () => void {
    return api.listen(LAN_SHARE_EVENTS.discovered, handler);
  }
}

export const lanShareAPI = new LanShareAPI();
