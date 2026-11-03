import { useState, useSyncExternalStore } from 'react';
import { Icon, IconButton, Tooltip } from '@openbitfun/ui';
import { useSceneStore } from '@/app/stores/sceneStore';
import { isSessionNavRowActive } from '@/app/components/NavPanel/sections/sessions/sessionNavSelection';
import { dispatchJobStore } from '@/features/dispatch/dispatchJobStore';
import {
  openSharedAssistantConversation,
  sharedAssistantConversations,
  startSharedAssistantConversation,
  type SharedAssistantRef,
} from '@/features/dispatch/sharedAssistantConversation';
import { sharedAssistantStatus, type SharedAssistantStatus } from '@/features/dispatch/sharedAssistantStatus';
import { flowChatStore } from '@/flow_chat/store/FlowChatStore';
import { openMainSession } from '@/flow_chat/services/sessionActivation';
import type { SharedWithMe } from '@/infrastructure/api/service-api/LanShareAPI';
import { useI18n } from '@/infrastructure/i18n';
import { useLanShareStore } from '@/infrastructure/lan-share/lanShareStore';
import { usePeerDeviceModeOptional } from '@/infrastructure/peer-device/peerDeviceContextState';
import { notificationService } from '@/shared/notification-system';
import { createLogger } from '@/shared/utils/logger';

const log = createLogger('SharedAssistantList');

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function refOf(assistant: SharedWithMe): SharedAssistantRef {
  return { ownerDeviceId: assistant.owner.deviceId, assistantId: assistant.assistantId, name: assistant.name };
}

/**
 * Assistants other devices share with this one, each with its conversations.
 * They run on their owner's device and belong to no workspace here, so they
 * have their own group. A connected peer's list is not this device's, so
 * nothing is listed in Peer Device Mode.
 */
export default function SharedAssistantList() {
  const { t } = useI18n('common');
  const peerActive = usePeerDeviceModeOptional()?.peerMode?.active === true;
  const sharedWithMe = useLanShareStore(state => state.sharedWithMe);
  const jobs = dispatchJobStore(state => state.jobs);
  const [busy, setBusy] = useState(false);
  const activeTabId = useSceneStore(state => state.activeTabId);
  const sessions = useSyncExternalStore(
    listener => flowChatStore.subscribe(() => listener()),
    () => flowChatStore.getState().sessions,
  );
  const activeSessionId = useSyncExternalStore(
    listener => flowChatStore.subscribe(() => listener()),
    () => flowChatStore.getState().activeSessionId,
  );

  if (peerActive || sharedWithMe.length === 0) return null;

  const run = (action: () => Promise<unknown>) => {
    if (busy) return;
    setBusy(true);
    void action()
      .catch((error) => {
        log.warn('Conversation with shared assistant not opened', { error: message(error) });
        notificationService.error(t('nav.sessions.sharedAssistant.openFailed', { reason: message(error) }));
      })
      .finally(() => setBusy(false));
  };

  const statusText = (status: SharedAssistantStatus, owner: string): string | null => {
    switch (status.kind) {
      case 'approval': return t('nav.sessions.sharedAssistant.status.approval', { owner });
      case 'queued': return status.limit
        ? t('nav.sessions.sharedAssistant.status.queuedLimit', { owner, limit: status.limit })
        : t('nav.sessions.sharedAssistant.status.queued');
      case 'running': return t('shared:statuses.running');
      case 'expired': return t('nav.sessions.sharedAssistant.status.expired');
      case 'failed': return t('shared:statuses.failed');
      default: return null;
    }
  };

  return (
    <div className="shared-assistant-nav" data-testid="nav-shared-assistants">
      <div className="shared-assistant-nav__title">{t('nav.sessions.sharedAssistant.title')}</div>
      {sharedWithMe.map((assistant) => {
        const ref = refOf(assistant);
        const owner = assistant.owner.displayName;
        const revoked = assistant.state === 'revoked';
        const standing = revoked
          ? t('nav.sessions.sharedAssistant.revoked')
          : assistant.online ? null : t('nav.sessions.sharedAssistant.offline');
        const source = [t('nav.sessions.sharedAssistant.from', { name: owner }), standing]
          .filter(Boolean).join(' · ');
        const conversations = sharedAssistantConversations(jobs, ref);
        return (
          <div key={`${assistant.owner.publicKey}/${assistant.assistantId}`} className="shared-assistant-nav__assistant" data-testid="nav-shared-assistant">
            <div className="shared-assistant-nav__header">
              <button
                type="button"
                className="openbitfun-nav-panel__inline-item shared-assistant-nav__name"
                data-openbitfun-component="sessions-section"
                data-openbitfun-part="row"
                title={source}
                aria-busy={busy}
                onClick={() => run(() => openSharedAssistantConversation(ref, !revoked))}
                data-testid="nav-shared-assistant-open"
              >
                <span className="assistant-session-history__delegated-title">{assistant.name}</span>
                <span className="assistant-session-history__delegated-detail">{source}</span>
              </button>
              {revoked ? null : (
                <Tooltip content={t('nav.sessions.sharedAssistant.newConversation')}>
                  <IconButton
                    size="xs"
                    variant="quiet"
                    aria-label={t('nav.sessions.sharedAssistant.newConversation')}
                    disabled={busy}
                    onClick={() => run(() => startSharedAssistantConversation(ref))}
                    icon={<Icon name="plus" size="sm" aria-hidden="true" />}
                    data-testid="nav-shared-assistant-new"
                  />
                </Tooltip>
              )}
            </div>
            {conversations.length > 0 ? (
              <div className="assistant-session-history__delegated">
                {conversations.map((job) => {
                  const status = sharedAssistantStatus(job);
                  const detail = statusText(status, owner);
                  const active = isSessionNavRowActive({
                    rowSessionId: job.sessionId,
                    activeTabId,
                    activeSessionId: activeSessionId ?? null,
                    activeChildHasVisibleRow: false,
                  });
                  const title = sessions.get(job.sessionId)?.title || job.title || t('nav.sessions.sharedAssistant.untitled');
                  return (
                    <button
                      key={job.jobId}
                      type="button"
                      className={[
                        'openbitfun-nav-panel__inline-item',
                        'assistant-session-history__delegated-item',
                        status.kind === 'approval' && 'is-waiting',
                        active && 'is-active',
                      ].filter(Boolean).join(' ')}
                      data-openbitfun-component="sessions-section"
                      data-openbitfun-part="row"
                      data-openbitfun-state={active ? 'active' : undefined}
                      aria-current={active ? 'true' : undefined}
                      title={detail ? `${title} · ${detail}` : title}
                      onClick={() => run(() => openMainSession(job.sessionId))}
                      data-testid="nav-shared-assistant-conversation"
                    >
                      <span className="assistant-session-history__delegated-title">{title}</span>
                      {detail ? <span className="assistant-session-history__delegated-detail">{detail}</span> : null}
                    </button>
                  );
                })}
              </div>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}
