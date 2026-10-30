import { api } from './ApiClient';
import { createTauriCommandError } from '../errors/TauriCommandError';

/** `inviter` or `acceptor`; unknown roles from newer builds are kept as strings. */
export type ConnectionRole = 'inviter' | 'acceptor' | (string & {});

/** One connection with a device of another account, as this device stores it. */
export interface ConnectionSummary {
  connectionId: string;
  role: ConnectionRole;
  /** `pending`, `active` or `revoked`, as last reported by the relay. */
  status: string;
  usable: boolean;
  /** Why the connection cannot carry requests, e.g. `key_changed`. */
  unusableReason?: string | null;
  /** This device invited and still has to confirm after checking the fingerprint. */
  awaitingConfirmation: boolean;
  peerUserId: string;
  peerDisplayName: string;
  peerDeviceId: string;
  peerDeviceName: string;
  peerFingerprint?: string | null;
  createdAt: number;
  confirmedAt?: number | null;
  revokedAt?: number | null;
}

export interface UnreadableConnection {
  connectionId?: string | null;
  error: string;
}

export interface OpenConnectionInvite {
  inviteId: string;
  deviceId: string;
  createdAt: number;
  expiresAt: number;
}

export interface ConnectionsOverview {
  /** The relay can route between accounts. */
  supported: boolean;
  /** Set when the relay state could not be read; local records are shown. */
  relayError?: string | null;
  localDeviceId: string;
  localFingerprint: string;
  connections: ConnectionSummary[];
  unreadable: UnreadableConnection[];
  invites: OpenConnectionInvite[];
}

export interface CreatedConnectionInvite {
  inviteId: string;
  link: string;
  fingerprint: string;
  expiresAt: number;
}

export interface ConnectionInvitePreview {
  inviterUserId: string;
  inviterDisplayName: string;
  inviterDeviceId: string;
  inviterDeviceName: string;
  /** Already checked against the fingerprint in the link. */
  fingerprint: string;
  expiresAt: number;
}

/** Stable reason codes the connection commands fail with. */
export const CONNECTION_ERROR_REASONS = [
  'relay_unsupported',
  'fingerprint_mismatch',
  'different_relay',
  'invalid_invite_link',
  'invite_not_found',
  'invite_expired',
  'invite_used',
  'same_account',
  'connection_exists',
  'connection_revoked',
  'connection_not_found',
  'inviter_device_required',
  'store_unreadable',
] as const;
export type ConnectionErrorReason = typeof CONNECTION_ERROR_REASONS[number];

/** The reason code an error carries, if it is one of the known ones. */
export function connectionErrorReason(error: unknown): ConnectionErrorReason | null {
  const message = (error instanceof Error ? error.message : String(error)).trim();
  return (CONNECTION_ERROR_REASONS as readonly string[]).includes(message)
    ? message as ConnectionErrorReason
    : null;
}

/** Connections of this device with devices of other accounts. */
class ConnectionsAPI {
  private async call<T>(command: string, request?: Record<string, unknown>): Promise<T> {
    try {
      return await api.invoke<T>(command, request ? { request } : undefined);
    } catch (error) {
      throw createTauriCommandError(command, error, request);
    }
  }

  overview(): Promise<ConnectionsOverview> {
    return this.call('connections_overview');
  }

  createInvite(expiresInSeconds?: number): Promise<CreatedConnectionInvite> {
    return this.call('connections_create_invite', { expiresInSeconds: expiresInSeconds ?? null });
  }

  previewInvite(link: string): Promise<ConnectionInvitePreview> {
    return this.call('connections_preview_invite', { link });
  }

  acceptInvite(link: string): Promise<ConnectionSummary> {
    return this.call('connections_accept_invite', { link });
  }

  confirm(connectionId: string, fingerprint: string): Promise<ConnectionSummary> {
    return this.call('connections_confirm', { connectionId, fingerprint });
  }

  revoke(connectionId: string): Promise<void> {
    return this.call('connections_revoke', { connectionId });
  }

  withdrawInvite(inviteId: string): Promise<void> {
    return this.call('connections_withdraw_invite', { inviteId });
  }
}

export const connectionsAPI = new ConnectionsAPI();
