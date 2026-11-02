import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Button, Input, Select } from '@openbitfun/ui';
import {
  connectionErrorReason,
  connectionsAPI,
  type ConnectionInvitePreview,
  type ConnectionsOverview,
  type ConnectionSummary,
  type CreatedConnectionInvite,
} from '@/infrastructure/api/service-api/ConnectionsAPI';
import {
  delegationAPI,
  type DelegationTrust,
  type OpenAgent,
} from '@/infrastructure/api/service-api/DelegationAPI';
import {
  notifyDelegationGrantsChanged,
  onDelegationGrantsChanged,
} from '@/infrastructure/api/service-api/delegationGrantChanges';
import { useI18n } from '@/infrastructure/i18n/hooks/useI18n';
import { usePeerDeviceModeOptional } from '@/infrastructure/peer-device/peerDeviceContextState';
import { isTauriRuntime } from '@/infrastructure/runtime';
import { notificationService } from '@/shared/notification-system';
import { writeNativeClipboardText } from '@/shared/utils/nativeClipboard';
import { createLogger } from '@/shared/utils/logger';
import {
  ConfigLoadingState,
  ConfigPageRow,
  ConfigPageSection,
  ConfigRetryState,
} from './common';

const log = createLogger('ConnectionsConfig');

type LoadState =
  | { kind: 'loading' }
  | { kind: 'signedOut' }
  | { kind: 'failed' }
  | {
    kind: 'ready';
    overview: ConnectionsOverview;
    trust: Record<string, DelegationTrust>;
    /** Opened assistants, to name those each account can use. */
    assistants: OpenAgent[];
  };

interface AcceptDraft {
  link: string;
  preview: ConnectionInvitePreview | null;
}

function isSignedOut(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error);
  return message.includes('not logged in');
}

/**
 * Connections of this device with devices of other accounts. Pairing uses
 * this device's own account identity, so it is managed only here: hosts
 * without the desktop commands and connected peer devices show an explicit
 * state. Nothing is shared over a connection until an assistant is opened
 * to it on that assistant's card; each row names the assistants it can use.
 */
export const ConnectionsConfig: React.FC = () => {
  const { t, formatDate } = useI18n('settings/runtime');
  const peerMode = usePeerDeviceModeOptional()?.peerMode;
  const supported = isTauriRuntime() && peerMode?.active !== true;
  const [state, setState] = useState<LoadState>({ kind: 'loading' });
  const [invite, setInvite] = useState<CreatedConnectionInvite | null>(null);
  const [accept, setAccept] = useState<AcceptDraft | null>(null);
  const [confirming, setConfirming] = useState<ConnectionSummary | null>(null);
  const [acceptedFingerprint, setAcceptedFingerprint] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  /** The connection whose full trust the owner is confirming. */
  const [trusting, setTrusting] = useState<string | null>(null);
  /** The connection whose revocation the owner is confirming. */
  const [revoking, setRevoking] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const activeRef = useRef(true);

  const load = useCallback(async () => {
    try {
      const [overview, trust, assistants] = await Promise.all([
        connectionsAPI.overview(),
        // Without trust records every connection is trusted normally.
        delegationAPI.listConnectionTrust().catch(() => ({} as Record<string, DelegationTrust>)),
        // Without opened assistants no account can use one.
        delegationAPI.listOpenAgents().catch(() => [] as OpenAgent[]),
      ]);
      if (activeRef.current) setState({ kind: 'ready', overview, trust, assistants });
    } catch (error) {
      if (!activeRef.current) return;
      if (isSignedOut(error)) {
        setState({ kind: 'signedOut' });
        return;
      }
      log.error('Failed to load connections', { error });
      setState({ kind: 'failed' });
    }
  }, []);

  useEffect(() => {
    activeRef.current = true;
    if (supported) void load();
    return () => { activeRef.current = false; };
  }, [load, supported]);

  // Assistants are opened to accounts on their own cards.
  useEffect(() => onDelegationGrantsChanged(source => {
    if (source !== activeRef && supported) void load();
  }), [load, supported]);

  const describeError = (error: unknown): string => {
    const reason = connectionErrorReason(error);
    if (reason) return t(`connections.errors.${reason}`);
    return t('connections.errors.generic', {
      message: error instanceof Error ? error.message : String(error),
    });
  };

  /** Runs an action, shows its error inline, and reloads the overview. */
  const run = async (action: () => Promise<void>, failureLog: string) => {
    setBusy(true);
    setActionError(null);
    try {
      await action();
      notifyDelegationGrantsChanged(activeRef);
      await load();
    } catch (error) {
      log.error(failureLog, { error });
      if (activeRef.current) setActionError(describeError(error));
    } finally {
      if (activeRef.current) setBusy(false);
    }
  };

  const formatTime = (seconds: number) => formatDate(seconds * 1000, {
    month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit',
  });

  const statusText = (connection: ConnectionSummary): string => {
    if (connection.awaitingConfirmation) return t('connections.status.awaitingYou');
    if (connection.unusableReason === 'key_changed') return t('connections.status.keyChanged');
    if (connection.unusableReason === 'not_verified') return t('connections.status.notVerified');
    if (connection.status === 'active') return t('connections.status.active');
    if (connection.status === 'pending') return t('connections.status.pending');
    if (connection.status === 'revoked') return t('connections.status.revoked');
    return connection.status;
  };

  const copyLink = async (link: string) => {
    const copied = await writeNativeClipboardText(link).catch(() => false)
      || await navigator.clipboard?.writeText(link).then(() => true, () => false);
    if (copied) notificationService.success(t('connections.invite.copied'), { duration: 2000 });
    else notificationService.error(t('connections.invite.copyFailed'));
  };

  const trustOf = (connectionId: string): DelegationTrust =>
    state.kind === 'ready' && state.trust[connectionId] === 'full' ? 'full' : 'normal';

  const setTrust = (connectionId: string, level: 'normal' | 'full') => void run(async () => {
    await delegationAPI.setConnectionTrust(connectionId, level, level === 'full');
    if (activeRef.current) setTrusting(null);
    notificationService.success(level === 'full'
      ? t('connections.trust.fullSaved')
      : t('connections.trust.normalSaved'), { duration: 4000 });
  }, 'Failed to set connection trust');

  const renderTrustConfirm = (connection: ConnectionSummary) => (
    <>
      <ConfigPageRow label={t('connections.trust.confirmTitle', { name: connection.peerDisplayName })}
        description={t('connections.trust.confirmDescription')}>
        <span />
      </ConfigPageRow>
      <ConfigPageRow label="" align="center" wide>
        <div className="openbitfun-runtime-settings__status-actions">
          <Button type="button" size="sm" variant="outline" disabled={busy} onClick={() => setTrusting(null)}>
            {t('connections.cancel')}
          </Button>
          <Button type="button" size="sm" variant="primary" disabled={busy}
            onClick={() => setTrust(connection.connectionId, 'full')}>
            {t('connections.trust.confirm')}
          </Button>
        </div>
      </ConfigPageRow>
    </>
  );

  const grantedAssistants = (connectionId: string): OpenAgent[] => state.kind === 'ready'
    ? state.assistants.filter(agent => agent.connections?.some(item => item.connectionId === connectionId))
    : [];

  const renderConnection = (connection: ConnectionSummary) => {
    const trust = trustOf(connection.connectionId);
    const live = connection.status !== 'revoked';
    const settled = connection.status === 'active' && connection.usable && !connection.awaitingConfirmation;
    const assistants = grantedAssistants(connection.connectionId);
    const details = [
      // A working connection says which assistants it can use; anything else says what is wrong.
      settled ? null : statusText(connection),
      settled || !connection.peerFingerprint
        ? null
        : t('connections.fingerprintLabel', { fingerprint: connection.peerFingerprint }),
      settled
        ? assistants.length > 0
          ? t('connections.assistantsLabel', { names: assistants.map(agent => agent.displayName).join('、') })
          : t('connections.noAssistants')
        : null,
    ].filter(Boolean).join(' · ');
    return (
      <ConfigPageRow
        key={connection.connectionId}
        balanced
        label={t('connections.peerDevice', {
          name: connection.peerDisplayName,
          device: connection.peerDeviceName || connection.peerDeviceId,
        })}
        description={details || undefined}
        align="center"
      >
        <div className="openbitfun-runtime-settings__status-actions">
          {connection.awaitingConfirmation && (
            <Button type="button" size="sm" variant="primary" disabled={busy}
              onClick={() => { setConfirming(connection); setActionError(null); }}>
              {t('connections.confirm.open')}
            </Button>
          )}
          {live && (
            <div className="openbitfun-runtime-settings__compact-select">
              <Select
                size="sm"
                value={trusting === connection.connectionId ? 'full' : trust}
                disabled={busy}
                aria-label={t('connections.trust.label')}
                options={[
                  { value: 'normal', label: t('connections.trust.normal') },
                  { value: 'full', label: t('connections.trust.full') },
                ]}
                onValueChange={value => {
                  const level = String(Array.isArray(value) ? value[0] : value);
                  setActionError(null);
                  if (level === 'full' && trust !== 'full') setTrusting(connection.connectionId);
                  else if (level === 'normal' && trusting === connection.connectionId) setTrusting(null);
                  else if (level === 'normal' && trust === 'full') setTrust(connection.connectionId, 'normal');
                }}
              />
            </div>
          )}
          {live && revoking !== connection.connectionId && (
            <Button type="button" size="sm" variant="outline" disabled={busy}
              onClick={() => { setRevoking(connection.connectionId); setActionError(null); }}>
              {t('connections.revoke')}
            </Button>
          )}
        </div>
      </ConfigPageRow>
    );
  };

  const renderRevokeConfirm = (connection: ConnectionSummary) => (
    <ConfigPageRow label={t('connections.revokeConfirmTitle', { name: connection.peerDisplayName })}
      description={t('connections.revokeConfirmDescription')} align="center">
      <div className="openbitfun-runtime-settings__status-actions">
        <Button type="button" size="sm" variant="outline" disabled={busy} onClick={() => setRevoking(null)}>
          {t('connections.cancel')}
        </Button>
        <Button type="button" size="sm" variant="primary" disabled={busy}
          onClick={() => void run(async () => {
            await connectionsAPI.revoke(connection.connectionId);
            if (activeRef.current) setRevoking(null);
          }, 'Failed to revoke connection')}>
          {t('connections.revoke')}
        </Button>
      </div>
    </ConfigPageRow>
  );

  const renderConfirm = (connection: ConnectionSummary) => (
    <>
      <ConfigPageRow
        balanced
        label={t('connections.confirm.title', { name: connection.peerDisplayName })}
        description={t('connections.confirm.description', { name: connection.peerDisplayName })}
      >
        <code className="openbitfun-runtime-settings__mono-value" data-testid="confirm-fingerprint">{connection.peerFingerprint ?? ''}</code>
      </ConfigPageRow>
      <ConfigPageRow label="" align="center" wide>
        <div className="openbitfun-runtime-settings__status-actions">
          <Button type="button" size="sm" variant="outline" disabled={busy} onClick={() => setConfirming(null)}>
            {t('connections.cancel')}
          </Button>
          <Button type="button" size="sm" variant="primary" disabled={busy || !connection.peerFingerprint}
            onClick={() => void run(async () => {
              await connectionsAPI.confirm(connection.connectionId, connection.peerFingerprint ?? '');
              if (activeRef.current) setConfirming(null);
            }, 'Failed to confirm connection')}>
            {t('connections.confirm.submit')}
          </Button>
        </div>
      </ConfigPageRow>
    </>
  );

  const renderAccept = (draft: AcceptDraft) => (
    <>
      <ConfigPageRow label={t('connections.accept.link')} description={t('connections.accept.linkDescription')} align="center">
        <Input size="sm" value={draft.link} disabled={busy || draft.preview !== null}
          placeholder="openbitfun://connect?..."
          onChange={event => setAccept({ link: event.target.value, preview: null })} />
      </ConfigPageRow>
      {draft.preview && (
        <ConfigPageRow
          balanced
          label={t('connections.peer', {
            name: draft.preview.inviterDisplayName,
            account: draft.preview.inviterUserId,
          })}
          description={t('connections.accept.previewDescription', {
            device: draft.preview.inviterDeviceName,
            time: formatTime(draft.preview.expiresAt),
          })}
        >
          <code className="openbitfun-runtime-settings__mono-value" data-testid="preview-fingerprint">{draft.preview.fingerprint}</code>
        </ConfigPageRow>
      )}
      <ConfigPageRow label="" align="center" wide>
        <div className="openbitfun-runtime-settings__status-actions">
          <Button type="button" size="sm" variant="outline" disabled={busy} onClick={() => setAccept(null)}>
            {t('connections.cancel')}
          </Button>
          {draft.preview ? (
            <Button type="button" size="sm" variant="primary" disabled={busy}
              onClick={() => void run(async () => {
                await connectionsAPI.acceptInvite(draft.link.trim());
                if (!activeRef.current) return;
                setAccept(null);
                setAcceptedFingerprint(state.kind === 'ready' ? state.overview.localFingerprint : null);
              }, 'Failed to accept invite')}>
              {t('connections.accept.submit')}
            </Button>
          ) : (
            <Button type="button" size="sm" variant="primary" disabled={busy || draft.link.trim() === ''}
              onClick={() => void run(async () => {
                const preview = await connectionsAPI.previewInvite(draft.link.trim());
                if (activeRef.current) setAccept({ link: draft.link, preview });
              }, 'Failed to read invite')}>
              {t('connections.accept.check')}
            </Button>
          )}
        </div>
      </ConfigPageRow>
    </>
  );

  const renderBody = () => {
    if (!supported) {
      return (
        <ConfigPageRow label={t('connections.sectionTitle')} description={peerMode?.active
          ? t('connections.peerUnsupported')
          : t('connections.unsupported')}>
          <span />
        </ConfigPageRow>
      );
    }
    if (state.kind === 'loading') return <ConfigLoadingState label={t('loading.text')} />;
    if (state.kind === 'failed') {
      return <ConfigRetryState message={t('loading.failed')} retryLabel={t('loading.retry')} onRetry={() => void load()} />;
    }
    if (state.kind === 'signedOut') {
      return <ConfigPageRow label={t('connections.signedOut')} align="center"><span /></ConfigPageRow>;
    }
    const { overview } = state;
    return <>
      {!overview.supported && (
        <ConfigPageRow label={t('connections.relayUnsupported')} description={t('connections.relayUnsupportedDescription')}>
          <span />
        </ConfigPageRow>
      )}
      {overview.relayError && (
        <ConfigPageRow label={t('connections.relayUnavailable')} description={overview.relayError}>
          <span />
        </ConfigPageRow>
      )}
      {acceptedFingerprint && (
        <ConfigPageRow balanced label={t('connections.accept.done')} description={t('connections.accept.doneDescription')}>
          <code className="openbitfun-runtime-settings__mono-value">{acceptedFingerprint}</code>
        </ConfigPageRow>
      )}
      {overview.connections.length === 0 && overview.unreadable.length === 0 && (
        <ConfigPageRow label={t('connections.empty')} align="center"><span /></ConfigPageRow>
      )}
      {overview.connections.map(connection =>
        confirming?.connectionId === connection.connectionId
          ? <React.Fragment key={connection.connectionId}>{renderConfirm(connection)}</React.Fragment>
          : (
            <React.Fragment key={connection.connectionId}>
              {renderConnection(connection)}
              {trusting === connection.connectionId && renderTrustConfirm(connection)}
              {revoking === connection.connectionId && renderRevokeConfirm(connection)}
            </React.Fragment>
          ))}
      {overview.unreadable.map((entry, index) => (
        <ConfigPageRow
          key={`unreadable-${entry.connectionId ?? index}`}
          label={entry.connectionId ?? t('connections.unreadableFile')}
          description={t('connections.unreadable', { problem: entry.error })}
        >
          <span />
        </ConfigPageRow>
      ))}
      {overview.invites.filter(open => open.inviteId !== invite?.inviteId).map(open => (
        <ConfigPageRow
          key={open.inviteId}
          label={t('connections.invite.open')}
          description={t('connections.invite.expires', { time: formatTime(open.expiresAt) })}
          align="center"
        >
          <Button type="button" size="sm" variant="outline" disabled={busy || !overview.supported}
            onClick={() => void run(async () => {
              await connectionsAPI.withdrawInvite(open.inviteId);
              if (activeRef.current && invite?.inviteId === open.inviteId) setInvite(null);
            }, 'Failed to withdraw invite')}>
            {t('connections.invite.withdraw')}
          </Button>
        </ConfigPageRow>
      ))}
      {invite && (
        <ConfigPageRow
          balanced
          label={t('connections.invite.created')}
          description={t('connections.invite.createdDescription', { time: formatTime(invite.expiresAt) })}
        >
          <div className="openbitfun-runtime-settings__status-actions">
            <code className="openbitfun-runtime-settings__mono-value" data-testid="invite-link">{invite.link}</code>
            <Button type="button" size="sm" variant="primary" onClick={() => void copyLink(invite.link)}>
              {t('connections.invite.copy')}
            </Button>
            <Button type="button" size="sm" variant="outline" disabled={busy}
              onClick={() => void run(async () => {
                await connectionsAPI.withdrawInvite(invite.inviteId);
                if (activeRef.current) setInvite(null);
              }, 'Failed to withdraw invite')}>
              {t('connections.invite.withdraw')}
            </Button>
          </div>
        </ConfigPageRow>
      )}
      {accept && renderAccept(accept)}
      {actionError && <div role="alert">{actionError}</div>}
      {!accept && (
        <ConfigPageRow label="" align="center" wide>
          <div className="openbitfun-runtime-settings__status-actions">
            <Button type="button" size="sm" variant="outline" disabled={busy || !overview.supported}
              onClick={() => void run(async () => {
                const created = await connectionsAPI.createInvite();
                if (activeRef.current) setInvite(created);
              }, 'Failed to create invite')}>
              {t('connections.invite.create')}
            </Button>
            <Button type="button" size="sm" variant="outline" disabled={busy || !overview.supported}
              onClick={() => { setAccept({ link: '', preview: null }); setActionError(null); }}>
              {t('connections.accept.open')}
            </Button>
          </div>
        </ConfigPageRow>
      )}
    </>;
  };

  return (
    <ConfigPageSection
      title={t('connections.sectionTitle')}
      description={t('connections.sectionDescription')}
    >
      {renderBody()}
    </ConfigPageSection>
  );
};

export default ConnectionsConfig;
