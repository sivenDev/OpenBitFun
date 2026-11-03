import React, { useEffect, useState } from 'react';
import {
  Alert,
  Button,
  Checkbox,
  Dialog,
  DialogBody,
  DialogClose,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogHeading,
  DialogTitle,
  Field,
  FieldGroup,
  FieldRow,
  NumberInput,
  SegmentedControl,
  Select,
} from '@openbitfun/ui';
import { useI18n } from '@/infrastructure/i18n/hooks/useI18n';
import { ACPClientAPI } from '@/infrastructure/api/service-api/ACPClientAPI';
import {
  lanShareAPI,
  type OwnedShare,
  type ShareSettings,
  type ShareTrust,
} from '@/infrastructure/api/service-api/LanShareAPI';
import { useLanShareStore } from '@/infrastructure/lan-share/lanShareStore';
import { useNotification } from '@/shared/notification-system';
import { createLogger } from '@/shared/utils/logger';
import './AssistantShareDialog.scss';

const log = createLogger('AssistantShareDialog');

const RETENTION_DAY_CHOICES = [1, 7, 30, 90] as const;
const MAX_CONCURRENT_TASKS = 100;
const DEFAULT_SHARE_SETTINGS: ShareSettings = {
  trust: 'normal',
  sessionRetentionDays: 7,
  maxConcurrentTasks: 2,
};

interface AssistantShareDialogProps {
  open: boolean;
  /** The assistant's directory. */
  assistantId: string;
  /** The assistant's name, as other devices will see it. */
  name: string;
  /** What other devices see when they look the assistant up. */
  description?: string;
  /** The current share, when changing its settings. */
  share?: OwnedShare;
  onClose: () => void;
}

/** Whether any external agent is set up on this device. */
async function hasExternalAgents(): Promise<boolean> {
  try {
    const clients = await ACPClientAPI.getClients();
    return clients.some((client) => client.enabled);
  } catch (error) {
    log.debug('External agents unavailable', { error });
    return false;
  }
}

/** Shares an assistant on the local network, or changes how it is shared. */
const AssistantShareDialog: React.FC<AssistantShareDialogProps> = ({
  open,
  assistantId,
  name,
  description,
  share,
  onClose,
}) => {
  const { t } = useI18n('scenes/profile');
  const notification = useNotification();
  const refreshOverview = useLanShareStore((state) => state.refreshOverview);
  const [settings, setSettings] = useState<ShareSettings>(DEFAULT_SHARE_SETTINGS);
  const [fullRiskAccepted, setFullRiskAccepted] = useState(false);
  const [externalAgentsNoticed, setExternalAgentsNoticed] = useState(false);
  const [showExternalAgents, setShowExternalAgents] = useState(false);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!open) return;
    setSettings(share
      ? {
          trust: share.trust,
          sessionRetentionDays: share.sessionRetentionDays,
          maxConcurrentTasks: share.maxConcurrentTasks,
        }
      : DEFAULT_SHARE_SETTINGS);
    setFullRiskAccepted(false);
    setExternalAgentsNoticed(false);
    setShowExternalAgents(false);
    if (share?.externalAgentUsageConfirmed) return;
    let active = true;
    void hasExternalAgents().then((present) => {
      if (active) setShowExternalAgents(present);
    });
    return () => { active = false; };
  }, [open, share]);

  const turningFull = settings.trust === 'full' && share?.trust !== 'full';
  const ready = (!turningFull || fullRiskAccepted) && (!showExternalAgents || externalAgentsNoticed);

  const handleSubmit = async () => {
    if (!ready || saving) return;
    setSaving(true);
    const confirmations = {
      fullTrustRisk: turningFull && fullRiskAccepted,
      externalAgentUsage: showExternalAgents && externalAgentsNoticed,
    };
    try {
      if (share) {
        await lanShareAPI.update(assistantId, { name, settings, confirmations });
        notification.success(t('nursery.share.dialog.saved'));
      } else {
        await lanShareAPI.share(assistantId, name, settings, confirmations, description || undefined);
        notification.success(t('nursery.share.dialog.shared', { name }));
      }
      void refreshOverview();
      onClose();
    } catch (error) {
      const reason = error instanceof Error ? error.message : String(error);
      log.error('Failed to save sharing settings', { assistantId, error: reason });
      notification.error(t(share ? 'nursery.share.dialog.saveFailed' : 'nursery.share.dialog.failed', { reason }));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog
      open={open}
      size="sm"
      onOpenChange={(nextOpen) => { if (!nextOpen) onClose(); }}
    >
      <DialogHeader>
        <DialogHeading>
          <DialogTitle>
            {t(share ? 'nursery.share.dialog.editTitle' : 'nursery.share.dialog.title', { name })}
          </DialogTitle>
          {share ? null : (
            <DialogDescription>{t('nursery.share.dialog.description')}</DialogDescription>
          )}
        </DialogHeading>
        <DialogClose />
      </DialogHeader>
      <DialogBody data-testid="assistant-share-dialog">
        <div className="assistant-share-dialog" data-openbitfun-component="assistant-share-dialog" data-openbitfun-part="fields">
        <FieldGroup appearance="subtle">
          <FieldRow padding="md">
            <div className="assistant-share-dialog__setting" data-openbitfun-component="assistant-share-dialog" data-openbitfun-part="setting">
              <Field
                orientation="horizontal"
                label={t('nursery.share.dialog.trust')}
                description={t(settings.trust === 'full'
                  ? 'nursery.share.dialog.trustFullHint'
                  : 'nursery.share.dialog.trustNormalHint')}
              >
                <SegmentedControl
                  size="sm"
                  value={settings.trust}
                  options={[
                    { value: 'normal', label: t('nursery.share.dialog.trustNormal') },
                    { value: 'full', label: t('nursery.share.dialog.trustFull') },
                  ]}
                  onValueChange={(value) => {
                    setSettings((current) => ({ ...current, trust: value as ShareTrust }));
                    setFullRiskAccepted(false);
                  }}
                />
              </Field>
              {turningFull ? (
                <Alert
                  tone="warning"
                  message={t('nursery.share.dialog.fullRiskWarning')}
                  description={(
                    <Checkbox
                      checked={fullRiskAccepted}
                      onCheckedChange={setFullRiskAccepted}
                      label={t('nursery.share.dialog.fullRiskConfirm')}
                      data-testid="assistant-share-full-risk"
                    />
                  )}
                />
              ) : null}
            </div>
          </FieldRow>
          <FieldRow padding="md">
            <Field orientation="horizontal" label={t('nursery.share.dialog.retention')}>
              <Select
                size="sm"
                className="assistant-share-dialog__control"
                value={String(settings.sessionRetentionDays)}
                options={RETENTION_DAY_CHOICES.map((days) => ({
                  value: String(days),
                  label: t('nursery.share.dialog.retentionDays', { count: days }),
                }))}
                onValueChange={(value) => setSettings((current) => ({
                  ...current,
                  sessionRetentionDays: Number(value),
                }))}
              />
            </Field>
          </FieldRow>
          <FieldRow padding="md">
            <Field
              orientation="horizontal"
              label={t('nursery.share.dialog.concurrency')}
              description={t('nursery.share.dialog.concurrencyHint')}
            >
              <NumberInput
                size="sm"
                variant="stepper"
                className="assistant-share-dialog__control"
                min={1}
                max={MAX_CONCURRENT_TASKS}
                step={1}
                value={settings.maxConcurrentTasks}
                onValueChange={(value) => setSettings((current) => ({
                  ...current,
                  maxConcurrentTasks: Math.min(MAX_CONCURRENT_TASKS, Math.max(1, Math.round(value))),
                }))}
              />
            </Field>
          </FieldRow>
        </FieldGroup>
        {showExternalAgents ? (
          <Alert
            tone="info"
            message={t('nursery.share.dialog.externalAgents')}
            description={(
              <Checkbox
                checked={externalAgentsNoticed}
                onCheckedChange={setExternalAgentsNoticed}
                label={t('nursery.share.dialog.externalAgentsConfirm')}
                data-testid="assistant-share-external-agents"
              />
            )}
          />
        ) : null}
        </div>
      </DialogBody>
      <DialogFooter>
        <Button size="sm" variant="fill" onClick={onClose}>
          {t('nursery.share.dialog.cancel')}
        </Button>
        <Button
          size="sm"
          variant="primary"
          disabled={!ready}
          loading={saving}
          onClick={() => { void handleSubmit(); }}
          data-testid="assistant-share-submit"
        >
          {t(share ? 'nursery.share.dialog.save' : 'nursery.share.dialog.submit')}
        </Button>
      </DialogFooter>
    </Dialog>
  );
};

export default AssistantShareDialog;
