import React, { useState } from 'react';
import {
  Button,
  Dialog,
  DialogBody,
  DialogClose,
  DialogFooter,
  DialogHeader,
  DialogHeading,
  DialogTitle,
} from '@openbitfun/ui';
import { useI18n } from '@/infrastructure/i18n/hooks/useI18n';
import { confirmDanger } from '@/infrastructure/confirm-dialog';
import { lanShareAPI, type OwnedShare, type ShareGrant } from '@/infrastructure/api/service-api/LanShareAPI';
import { useLanShareStore } from '@/infrastructure/lan-share/lanShareStore';
import { useNotification } from '@/shared/notification-system';
import { createLogger } from '@/shared/utils/logger';
import './AssistantShareDialog.scss';

const log = createLogger('AssistantShareManageDialog');

interface AssistantShareManageDialogProps {
  open: boolean;
  name: string;
  share: OwnedShare;
  onClose: () => void;
  /** Opens the sharing settings. */
  onEdit: () => void;
}

/** Who uses a shared assistant, and stopping or narrowing the share. */
const AssistantShareManageDialog: React.FC<AssistantShareManageDialogProps> = ({
  open,
  name,
  share,
  onClose,
  onEdit,
}) => {
  const { t, formatDate } = useI18n('scenes/profile');
  const notification = useNotification();
  const refreshOverview = useLanShareStore((state) => state.refreshOverview);
  const [busy, setBusy] = useState<string | null>(null);

  const run = async (key: string, action: () => Promise<unknown>) => {
    setBusy(key);
    try {
      await action();
      void refreshOverview();
    } catch (error) {
      const reason = error instanceof Error ? error.message : String(error);
      log.error('Sharing change failed', { assistantId: share.id, action: key, error: reason });
      notification.error(t('nursery.share.manage.failed', { reason }));
    } finally {
      setBusy(null);
    }
  };

  const when = (time: string) => formatDate(new Date(time), {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  });

  const handleRemove = async (grant: ShareGrant) => {
    const confirmed = await confirmDanger(
      t('nursery.share.manage.removeTitle', { name: grant.displayName }),
      t('nursery.share.manage.removeMessage', { name: grant.displayName, device: grant.deviceName }),
      { confirmText: t('nursery.share.manage.remove') },
    );
    if (!confirmed) return;
    await run(`remove:${grant.publicKey}`, () => lanShareAPI.revoke(share.id, grant.publicKey));
  };

  const handleDeleteData = async (deviceId: string) => {
    const confirmed = await confirmDanger(
      t('nursery.share.manage.deleteDataTitle'),
      t('nursery.share.manage.deleteDataMessage'),
      { confirmText: t('nursery.share.manage.deleteData') },
    );
    if (!confirmed) return;
    await run(`delete:${deviceId}`, () => lanShareAPI.deleteTenant(share.id, deviceId));
  };

  const handleStop = async () => {
    const confirmed = await confirmDanger(
      t('nursery.share.manage.stopTitle', { name }),
      t('nursery.share.manage.stopMessage'),
      { confirmText: t('nursery.share.manage.stop') },
    );
    if (!confirmed) return;
    await run('stop', () => lanShareAPI.stop(share.id));
    onClose();
  };

  const grantedDevices = new Set(share.grants.map((grant) => grant.deviceId));
  const leftDevices = share.tenantDeviceIds.filter((deviceId) => !grantedDevices.has(deviceId));
  const trustLabel = t(share.trust === 'full'
    ? 'nursery.share.dialog.trustFull'
    : 'nursery.share.dialog.trustNormal');

  return (
    <Dialog open={open} size="sm" onOpenChange={(nextOpen) => { if (!nextOpen) onClose(); }}>
      <DialogHeader>
        <DialogHeading>
          <DialogTitle>{t('nursery.share.manage.title', { name })}</DialogTitle>
        </DialogHeading>
        <DialogClose />
      </DialogHeader>
      <DialogBody className="assistant-share-manage" data-testid="assistant-share-manage">
        <p className="assistant-share-manage__summary" data-openbitfun-component="assistant-share-manage" data-openbitfun-part="summary">
          {t('nursery.share.manage.summary', {
            trust: trustLabel,
            days: share.sessionRetentionDays,
            tasks: share.maxConcurrentTasks,
          })}
        </p>

        <section className="assistant-share-manage__section" data-openbitfun-component="assistant-share-manage" data-openbitfun-part="section">
          <h3 className="assistant-share-manage__heading" data-openbitfun-component="assistant-share-manage" data-openbitfun-part="heading">{t('nursery.share.manage.users')}</h3>
          {share.grants.length === 0 ? (
            <p className="assistant-share-manage__empty" data-openbitfun-component="assistant-share-manage" data-openbitfun-part="empty">{t('nursery.share.manage.noUsers')}</p>
          ) : (
            <ul className="assistant-share-manage__list" data-openbitfun-component="assistant-share-manage" data-openbitfun-part="list">
              {share.grants.map((grant) => (
                <li key={grant.publicKey} className="assistant-share-manage__row" data-openbitfun-component="assistant-share-manage" data-openbitfun-part="row" data-testid="assistant-share-user">
                  <span className="assistant-share-manage__who" data-openbitfun-component="assistant-share-manage" data-openbitfun-part="who">
                    <span>{grant.displayName}</span>
                    <span className="assistant-share-manage__meta" data-openbitfun-component="assistant-share-manage" data-openbitfun-part="meta">
                      {t('nursery.share.manage.device', { device: grant.deviceName, code: grant.fingerprint })}
                      {' · '}
                      {grant.lastUsedAt
                        ? t('nursery.share.manage.lastUsed', { time: when(grant.lastUsedAt) })
                        : t('nursery.share.manage.grantedAt', { time: when(grant.grantedAt) })}
                    </span>
                  </span>
                  <Button
                    size="sm"
                    variant="fill"
                    tone="danger"
                    loading={busy === `remove:${grant.publicKey}`}
                    disabled={busy !== null}
                    onClick={() => { void handleRemove(grant); }}
                  >
                    {t('nursery.share.manage.remove')}
                  </Button>
                </li>
              ))}
            </ul>
          )}
        </section>

        {leftDevices.length > 0 ? (
          <section className="assistant-share-manage__section" data-openbitfun-component="assistant-share-manage" data-openbitfun-part="section">
            <h3 className="assistant-share-manage__heading" data-openbitfun-component="assistant-share-manage" data-openbitfun-part="heading">{t('nursery.share.manage.leftData')}</h3>
            <ul className="assistant-share-manage__list" data-openbitfun-component="assistant-share-manage" data-openbitfun-part="list">
              {leftDevices.map((deviceId) => (
                <li key={deviceId} className="assistant-share-manage__row" data-openbitfun-component="assistant-share-manage" data-openbitfun-part="row" data-testid="assistant-share-left-device">
                  <span className="assistant-share-manage__meta" data-openbitfun-component="assistant-share-manage" data-openbitfun-part="meta">
                    {t('nursery.share.manage.leftDevice', { id: deviceId.slice(0, 8) })}
                  </span>
                  <Button
                    size="sm"
                    variant="fill"
                    tone="danger"
                    loading={busy === `delete:${deviceId}`}
                    disabled={busy !== null}
                    onClick={() => { void handleDeleteData(deviceId); }}
                  >
                    {t('nursery.share.manage.deleteData')}
                  </Button>
                </li>
              ))}
            </ul>
          </section>
        ) : null}
      </DialogBody>
      <DialogFooter className="assistant-share-manage__footer">
        <Button
          size="sm"
          variant="fill"
          tone="danger"
          loading={busy === 'stop'}
          disabled={busy !== null}
          onClick={() => { void handleStop(); }}
          data-testid="assistant-share-stop"
        >
          {t('nursery.share.manage.stop')}
        </Button>
        <Button size="sm" variant="primary" disabled={busy !== null} onClick={onEdit}>
          {t('nursery.share.manage.edit')}
        </Button>
      </DialogFooter>
    </Dialog>
  );
};

export default AssistantShareManageDialog;
