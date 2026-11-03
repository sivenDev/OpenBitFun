import React, { useState } from 'react';
import {
  Button,
  Card,
  CardFooter,
  CardHeader,
  Dialog,
  DialogBody,
  DialogClose,
  DialogFooter,
  DialogHeader,
  DialogHeading,
  DialogTitle,
  Icon,
  IconButton,
  OverflowText,
  StatusPill,
  Tooltip,
} from '@openbitfun/ui';
import { AssistantAvatar } from '@/app/components/AssistantAvatar';
import { useI18n } from '@/infrastructure/i18n/hooks/useI18n';
import { confirmDanger } from '@/infrastructure/confirm-dialog';
import { lanShareAPI, type SharedWithMe } from '@/infrastructure/api/service-api/LanShareAPI';
import { useLanShareStore } from '@/infrastructure/lan-share/lanShareStore';
import { useNotification } from '@/shared/notification-system';
import { usePeerDeviceModeOptional } from '@/infrastructure/peer-device/peerDeviceContextState';
import {
  forgetSharedAssistantConversations,
  openSharedAssistantConversation,
  startSharedAssistantConversation,
  type SharedAssistantRef,
} from '@/features/dispatch/sharedAssistantConversation';
import { createLogger } from '@/shared/utils/logger';
import './SharedAssistantCard.scss';

const log = createLogger('SharedAssistantCard');

interface SharedAssistantCardProps {
  assistant: SharedWithMe;
  style?: React.CSSProperties;
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * An assistant another device shares with this one, marked with its source.
 * Opening it shows the latest conversation with it, or starts one.
 */
const SharedAssistantCard: React.FC<SharedAssistantCardProps> = ({ assistant, style }) => {
  const { t, formatDate } = useI18n('scenes/profile');
  const notification = useNotification();
  const [details, setDetails] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [opening, setOpening] = useState(false);
  const peerActive = usePeerDeviceModeOptional()?.peerMode.active ?? false;
  const revoked = assistant.state === 'revoked';
  const offline = !revoked && !assistant.online;
  const source = t('nursery.sharedWithMe.from', { name: assistant.owner.displayName });
  const ref: SharedAssistantRef = {
    ownerDeviceId: assistant.owner.deviceId,
    assistantId: assistant.assistantId,
    name: assistant.name,
  };

  const converse = async (fresh: boolean) => {
    if (opening) return;
    setOpening(true);
    try {
      const shown = fresh
        ? await startSharedAssistantConversation(ref)
        : await openSharedAssistantConversation(ref, !revoked);
      // Nothing to show: no conversation yet and none can start.
      if (!shown) setDetails(true);
    } catch (error) {
      log.error('Conversation with shared assistant not opened', {
        assistantId: assistant.assistantId,
        error: message(error),
      });
      notification.error(t('nursery.sharedWithMe.openFailed', { reason: message(error) }));
    } finally {
      setOpening(false);
    }
  };

  const remove = async () => {
    const confirmed = await confirmDanger(
      t('nursery.sharedWithMe.removeTitle', { name: assistant.name }),
      t(revoked ? 'nursery.sharedWithMe.removeRevokedMessage' : 'nursery.sharedWithMe.removeMessage', {
        owner: assistant.owner.displayName,
      }),
      { confirmText: t('nursery.sharedWithMe.remove') },
    );
    if (!confirmed) return;
    setRemoving(true);
    try {
      await lanShareAPI.remove(assistant.owner.publicKey, assistant.assistantId);
      setDetails(false);
      await forgetSharedAssistantConversations(ref);
      void useLanShareStore.getState().refreshSharedWithMe();
    } catch (error) {
      const reason = message(error);
      log.error('Shared assistant not removed', { assistantId: assistant.assistantId, error: reason });
      notification.error(t('nursery.sharedWithMe.removeFailed', { reason }));
    } finally {
      setRemoving(false);
    }
  };

  return (
    <article
      className={['assistant-card', 'shared-assistant-card', revoked && 'shared-assistant-card--revoked']
        .filter(Boolean).join(' ')}
      role="listitem"
      style={style}
      data-testid="shared-assistant-card"
      data-openbitfun-component="shared-assistant-card"
      data-openbitfun-part="root"
    >
      <Card className="assistant-card__surface" appearance="subtle" radius="md" padding="md" gap="sm" clip>
        <button
          type="button"
          className="assistant-card__main" data-openbitfun-component="shared-assistant-card" data-openbitfun-part="main"
          onClick={() => { if (peerActive) setDetails(true); else void converse(false); }}
          aria-label={`${assistant.name}, ${source}`}
          aria-busy={opening}
          data-testid="shared-assistant-open"
        />
        <CardHeader
          align="center"
          contentAlign="start"
          className="assistant-card__header"
          leading={(
            <span className="assistant-card__avatar" data-openbitfun-component="shared-assistant-card" data-openbitfun-part="avatar">
              <AssistantAvatar
                stableKey={`${assistant.owner.publicKey}/${assistant.assistantId}`}
                name={assistant.name}
                size={44}
              />
            </span>
          )}
          title={(
            <span className="assistant-card__title-row" data-openbitfun-component="shared-assistant-card" data-openbitfun-part="title">
              <OverflowText className="assistant-card__name">{assistant.name}</OverflowText>
              <span className="shared-assistant-card__badge" data-openbitfun-component="shared-assistant-card" data-openbitfun-part="badge">
                <StatusPill tone={revoked ? 'warning' : offline ? 'neutral' : 'info'}>
                  {revoked
                    ? t('nursery.sharedWithMe.revoked')
                    : offline
                      ? t('nursery.sharedWithMe.offline')
                      : t('nursery.sharedWithMe.badge')}
                </StatusPill>
              </span>
            </span>
          )}
          description={(
            <span className="assistant-card__metadata" data-openbitfun-component="shared-assistant-card" data-openbitfun-part="metadata">
              <OverflowText className="assistant-card__vibe">{source}</OverflowText>
            </span>
          )}
        />
        <CardFooter align="end" className="assistant-card__footer">
          <span className="assistant-card__footer-actions" data-openbitfun-component="shared-assistant-card" data-openbitfun-part="actions">
            {revoked ? null : (
              <Tooltip content={peerActive ? t('nursery.share.peerUnavailable') : t('nursery.sharedWithMe.newConversation')}>
                <IconButton
                  size="sm"
                  aria-label={t('nursery.sharedWithMe.newConversation')}
                  disabled={peerActive || opening}
                  onClick={() => { void converse(true); }}
                  icon={<Icon name="plus" size="sm" aria-hidden="true" />}
                  data-testid="shared-assistant-new-conversation"
                />
              </Tooltip>
            )}
            <Tooltip content={t('nursery.sharedWithMe.details')}>
              <IconButton
                size="sm"
                aria-label={t('nursery.sharedWithMe.details')}
                onClick={() => setDetails(true)}
                icon={<Icon name="info" size="sm" aria-hidden="true" />}
                data-testid="shared-assistant-details-open"
              />
            </Tooltip>
            <Tooltip content={t('nursery.sharedWithMe.remove')}>
              <IconButton
                size="sm"
                tone="danger"
                aria-label={t('nursery.sharedWithMe.remove')}
                disabled={removing}
                onClick={() => { void remove(); }}
                icon={<Icon name="delete" size="sm" aria-hidden="true" />}
                data-testid="shared-assistant-remove"
              />
            </Tooltip>
          </span>
        </CardFooter>
      </Card>

      {details ? (
        <Dialog open size="sm" onOpenChange={(nextOpen) => { if (!nextOpen) setDetails(false); }}>
          <DialogHeader>
            <DialogHeading>
              <DialogTitle>{assistant.name}</DialogTitle>
            </DialogHeading>
            <DialogClose />
          </DialogHeader>
          <DialogBody className="shared-assistant-card__details" data-testid="shared-assistant-details">
            {assistant.description ? <p>{assistant.description}</p> : null}
            <p className="shared-assistant-card__meta" data-openbitfun-component="shared-assistant-card" data-openbitfun-part="meta">
              {t('nursery.sharedWithMe.source', {
                name: assistant.owner.displayName,
                device: assistant.owner.deviceName,
                code: assistant.owner.fingerprint,
              })}
            </p>
            <p className="shared-assistant-card__meta" data-openbitfun-component="shared-assistant-card" data-openbitfun-part="meta">
              {t('nursery.sharedWithMe.addedAt', {
                time: formatDate(new Date(assistant.grantedAt), { year: 'numeric', month: 'short', day: 'numeric' }),
              })}
            </p>
            <p className="shared-assistant-card__meta" data-openbitfun-component="shared-assistant-card" data-openbitfun-part="meta">
              {revoked ? t('nursery.sharedWithMe.revokedHint') : t('nursery.discover.privacy')}
            </p>
            {peerActive ? (
              <p className="shared-assistant-card__meta" data-openbitfun-component="shared-assistant-card" data-openbitfun-part="meta">
                {t('nursery.sharedWithMe.peerUnavailable')}
              </p>
            ) : null}
          </DialogBody>
          <DialogFooter>
            <Button size="sm" variant="fill" tone="danger" loading={removing} onClick={() => { void remove(); }}>
              {t('nursery.sharedWithMe.remove')}
            </Button>
          </DialogFooter>
        </Dialog>
      ) : null}
    </article>
  );
};

export default SharedAssistantCard;
