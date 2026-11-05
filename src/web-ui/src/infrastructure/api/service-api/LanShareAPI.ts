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
  /** The same devices with who they belonged to. Absent from older builds. */
  tenantDevices?: TenantDevice[];
}

/** A device whose data under a shared assistant is kept. */
export interface TenantDevice {
  deviceId: string;
  displayName?: string;
  deviceName?: string;
}

/** This device as other devices see it. */
export interface LanShareHostStatus {
  deviceId: string;
  deviceName: string;
  displayName: string;
  fingerprint: string;
  /** Set while this device shares an assistant and listens. */
  port?: number;
  /**
   * What people on another subnet type to add this device, one per network;
   * empty while this device does not listen.
   */
  addresses: string[];
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
  | { state: 'unreachable'; message: string }
  /** Whoever answers at the device's address holds another key. */
  | { state: 'identityChanged'; message: string };

/** A sharing device on the local network. */
export interface DiscoveredDevice {
  deviceId: string;
  deviceName: string;
  displayName: string;
  fingerprint: string;
  publicKey: string;
  status: DiscoveredDeviceStatus;
  assistants: DiscoveredAssistant[];
  /** Set when the device was added by address (`ip:port`). */
  addedAddress?: string;
}

/** Why an address cannot be used to add a device. */
export type AddressProblem =
  | 'format'
  | 'hostName'
  | 'ipv6'
  | 'notLocal'
  | 'thisDevice'
  | 'unreachable'
  | 'notOpenBitFun'
  | 'needsUpdate'
  | 'identityChanged';

/** A device reached at an address someone typed. */
export interface AddressedDevice {
  /** Where it answered (`ip:port`). */
  address: string;
  deviceId: string;
  deviceName: string;
  displayName: string;
  /** The code the person compares with the one the device shows. */
  fingerprint: string;
  publicKey: string;
  /** How many assistants it shares. */
  assistants: number;
  /** Whether it was already added by address. */
  added: boolean;
}

export type AddressOutcome =
  | { state: 'reached'; device: AddressedDevice }
  | { state: 'problem'; problem: AddressProblem; message: string };

/** The port sharing devices listen on unless another program holds it. */
export const DEFAULT_LAN_SHARE_PORT = 47652;

/** An address as people read it: without the port when it is the default. */
export function shownAddress(address: string): string {
  const suffix = `:${DEFAULT_LAN_SHARE_PORT}`;
  return address.endsWith(suffix) ? address.slice(0, -suffix.length) : address;
}

/** The answer to a request: open until `deadlineMs`, or already decided. */
export interface ShareRequestOpened {
  requestId: string;
  deadlineMs: number;
  granted?: boolean;
}

/** What an owner device does for this device's conversations with its assistants. */
export interface SharedConversationSupport {
  deleteConversation: boolean;
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
  /** Whether its owner can be reached now. */
  online: boolean;
}

/** A job this device started on a shared assistant has news. */
export interface SharedJobUpdated {
  ownerKey: string;
  jobId: string;
}

export const LAN_SHARE_EVENTS = {
  requests: 'lan_share://requests',
  shared: 'lan_share://shared',
  sharedWithMe: 'lan_share://shared_with_me',
  discovered: 'lan_share://discovered',
  jobUpdated: 'lan_share://job_updated',
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

  /** Lets a device the owner blocked ask for the assistant again. */
  unblock(assistantId: string, publicKey: string): Promise<void> {
    return invoke('lan_share_unblock', { assistantId, publicKey });
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

  /** What the owner device does for this device's conversations; asks the owner. */
  conversationSupport(ownerDeviceId: string): Promise<SharedConversationSupport> {
    return invoke('lan_share_conversation_support', { ownerDeviceId });
  }

  /** Deletes a finished conversation with a shared assistant on its owner's device and here. */
  deleteConversation(jobId: string): Promise<void> {
    return invoke('lan_share_delete_conversation', { jobId });
  }

  /** Says who answers at an address someone typed. Nothing is kept. */
  probeAddress(address: string): Promise<AddressOutcome> {
    return invoke('lan_share_probe_address', { address });
  }

  /** Adds the device holding `publicKey` at `address`, after its code was compared. */
  addDevice(address: string, publicKey: string): Promise<AddressOutcome> {
    return invoke('lan_share_add_device', { address, publicKey });
  }

  /** Forgets a device added by address. */
  removeDevice(deviceId: string): Promise<boolean> {
    return invoke('lan_share_remove_device', { deviceId });
  }

  /** Moves a device added by address; the device there must be the same one. */
  setDeviceAddress(deviceId: string, address: string): Promise<AddressOutcome> {
    return invoke('lan_share_set_device_address', { deviceId, address });
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

  onJobUpdated(handler: (payload: SharedJobUpdated) => void): () => void {
    return api.listen(LAN_SHARE_EVENTS.jobUpdated, handler);
  }
}

export const lanShareAPI = new LanShareAPI();
