import { create } from 'zustand';
import {
  lanShareAPI,
  type DiscoveredDevice,
  type LanShareOverview,
  type OwnedShare,
  type ShareAccess,
  type ShareRequest,
  type SharedWithMe,
} from '@/infrastructure/api/service-api/LanShareAPI';
import { createLogger } from '@/shared/utils/logger';

const log = createLogger('LanShareStore');

interface LanShareState {
  overview: LanShareOverview | null;
  /** Why sharing on the local network is unavailable on this device. */
  unavailable: string | null;
  requests: ShareRequest[];
  sharedWithMe: SharedWithMe[];
  /** Whether `sharedWithMe` has been read since startup. */
  sharedWithMeLoaded: boolean;
  discovered: DiscoveredDevice[];
  refreshOverview: () => Promise<void>;
  refreshRequests: () => Promise<void>;
  refreshSharedWithMe: () => Promise<void>;
  refreshDiscovered: () => Promise<void>;
  setRequests: (requests: ShareRequest[]) => void;
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export const useLanShareStore = create<LanShareState>((set) => ({
  overview: null,
  unavailable: null,
  requests: [],
  sharedWithMe: [],
  sharedWithMeLoaded: false,
  discovered: [],
  refreshOverview: async () => {
    try {
      set({ overview: await lanShareAPI.overview(), unavailable: null });
    } catch (error) {
      log.warn('LAN sharing overview unavailable', { error: message(error) });
      set({ overview: null, unavailable: message(error) });
    }
  },
  refreshRequests: async () => {
    try {
      set({ requests: await lanShareAPI.requests() });
    } catch (error) {
      log.warn('LAN sharing requests unavailable', { error: message(error) });
    }
  },
  refreshSharedWithMe: async () => {
    try {
      set({ sharedWithMe: await lanShareAPI.sharedWithMe(), sharedWithMeLoaded: true });
    } catch (error) {
      log.warn('Assistants shared with this device unavailable', { error: message(error) });
    }
  },
  refreshDiscovered: async () => {
    try {
      set({ discovered: await lanShareAPI.discovered() });
    } catch (error) {
      log.warn('Discovered devices unavailable', { error: message(error) });
    }
  },
  setRequests: (requests) => set({ requests }),
}));

let subscribers = 0;
let unsubscribe: (() => void) | null = null;

/**
 * Keeps the store current from the host's events while at least one view
 * needs it. Returns the release function.
 */
export function subscribeLanShare(): () => void {
  subscribers += 1;
  if (subscribers === 1) {
    const store = useLanShareStore.getState();
    const releases = [
      lanShareAPI.onRequests((payload) => {
        useLanShareStore.getState().setRequests(payload?.requests ?? []);
      }),
      lanShareAPI.onShared(() => { void useLanShareStore.getState().refreshOverview(); }),
      lanShareAPI.onSharedWithMe(() => { void useLanShareStore.getState().refreshSharedWithMe(); }),
      lanShareAPI.onDiscovered(() => { void useLanShareStore.getState().refreshDiscovered(); }),
    ];
    unsubscribe = () => releases.forEach((release) => release());
    void store.refreshOverview();
    void store.refreshRequests();
    void store.refreshSharedWithMe();
  }
  let released = false;
  return () => {
    if (released) return;
    released = true;
    subscribers -= 1;
    if (subscribers === 0) {
      unsubscribe?.();
      unsubscribe = null;
    }
  };
}

/** The assistant's directory, the id a share is known by. */
export function assistantShareId(rootPath: string): string {
  const parts = rootPath.split(/[\\/]+/).filter(Boolean);
  return parts[parts.length - 1] ?? '';
}

/** The share of the assistant whose workspace is `rootPath`. */
export function shareForWorkspace(
  overview: LanShareOverview | null,
  rootPath: string,
): OwnedShare | undefined {
  if (!overview) return undefined;
  const id = assistantShareId(rootPath);
  return overview.shares.find((share) => share.rootPath === rootPath || share.id === id);
}

/**
 * Where this device stands with a discovered assistant at `nowMs`: a
 * cooldown that has passed means it may ask again, even before the owner's
 * catalog is read again.
 */
export function currentAccess(access: ShareAccess, nowMs: number): ShareAccess {
  if (access.state === 'cooling' && access.retryAtMs <= nowMs) return { state: 'available' };
  return access;
}

/** Seconds left until `deadlineMs`, never negative. */
export function secondsLeft(deadlineMs: number, nowMs: number): number {
  return Math.max(0, Math.ceil((deadlineMs - nowMs) / 1000));
}

/** Why this device cannot send to a shared assistant now, if it cannot. */
export type SharedAssistantBlock = 'notShared' | 'revoked' | 'offline';

/**
 * Whether this device can send to `assistantId` of `ownerDeviceId` now,
 * from the assistants shared with it. Unknown until the list is read.
 */
export function sharedAssistantBlock(
  state: Pick<LanShareState, 'sharedWithMe' | 'sharedWithMeLoaded'>,
  ownerDeviceId: string,
  assistantId: string,
): SharedAssistantBlock | undefined {
  if (!state.sharedWithMeLoaded) return undefined;
  const shared = state.sharedWithMe.find((entry) =>
    entry.owner.deviceId === ownerDeviceId && entry.assistantId === assistantId);
  if (!shared) return 'notShared';
  if (shared.state === 'revoked') return 'revoked';
  return shared.online ? undefined : 'offline';
}
