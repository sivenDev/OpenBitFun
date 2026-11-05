import { useId, useRef, useState, useSyncExternalStore } from 'react';
import { Icon, IconButton, MenuPopover, Tooltip, type MenuEntry } from '@openbitfun/ui';
import { Ban, ChevronUp, History, WifiOff } from 'lucide-react';
import { useSceneStore } from '@/app/stores/sceneStore';
import { isSessionNavRowActive } from '@/app/components/NavPanel/sections/sessions/sessionNavSelection';
import { dispatchJobStore, type DispatchObserverJob } from '@/features/dispatch/dispatchJobStore';
import {
  deleteSharedAssistantConversation,
  isLocalOnlyConversation,
  openSharedAssistantConversation,
  openSharedAssistantConversationJob,
  sharedAssistantConversations,
  sharedConversationDeleteSupported,
  startSharedAssistantConversation,
  type SharedAssistantRef,
} from '@/features/dispatch/sharedAssistantConversation';
import { explainSharedAssistantError } from '@/features/dispatch/sharedAssistantProblemText';
import { sharedAssistantStatus, type SharedAssistantStatus } from '@/features/dispatch/sharedAssistantStatus';
import { flowChatStore } from '@/flow_chat/store/FlowChatStore';
import { confirmDanger } from '@/infrastructure/confirm-dialog';
import type { SharedWithMe } from '@/infrastructure/api/service-api/LanShareAPI';
import { useI18n } from '@/infrastructure/i18n';
import { useLanShareStore } from '@/infrastructure/lan-share/lanShareStore';
import { usePeerDeviceModeOptional } from '@/infrastructure/peer-device/peerDeviceContextState';
import { notificationService } from '@/shared/notification-system';
import { createLogger } from '@/shared/utils/logger';
import { limitRows } from './delegatedTaskGroups';
import { NavTaskStatusIcon } from './NavTaskStatus';
import { fullLists, openedGroups } from './navListMemory';

const log = createLogger('SharedAssistantList');

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function refOf(assistant: SharedWithMe): SharedAssistantRef {
  return { ownerDeviceId: assistant.owner.deviceId, assistantId: assistant.assistantId, name: assistant.name };
}

/** Why a conversation cannot be deleted now, or `null` when it can. */
type DeleteBlock = 'offline' | 'revoked' | 'active' | 'needsUpdate' | 'checking';

/** Whether the owner deletes conversations, as far as this menu knows. */
type OwnerSupport = 'checking' | 'yes' | 'no' | 'unreachable';

interface ConversationActionsProps {
  job: DispatchObserverJob;
  assistant: SharedAssistantRef;
  owner: string;
  title: string;
  revoked: boolean;
  online: boolean;
  /** The conversation waits on the owner or runs there. */
  active: boolean;
}

/**
 * The "..." menu of one conversation with a shared assistant. Deleting
 * removes it on both devices, so the owner must be reachable, still share
 * the assistant, support it, and not be running the conversation; a new
 * conversation that never reached the owner is removed here at once.
 */
function SharedConversationActions({ job, assistant, owner, title, revoked, online, active }: ConversationActionsProps) {
  const { t } = useI18n('common');
  const [open, setOpen] = useState(false);
  const [support, setSupport] = useState<OwnerSupport>('checking');
  const [deleting, setDeleting] = useState(false);
  const anchorRef = useRef<HTMLButtonElement>(null);
  const menuId = useId();
  const localOnly = isLocalOnlyConversation(job);
  const moreLabel = t('nav.sessions.sharedAssistant.more');

  const block: DeleteBlock | null = localOnly ? null
    : revoked ? 'revoked'
    : !online || support === 'unreachable' ? 'offline'
    : active ? 'active'
    : support === 'checking' ? 'checking'
    : support === 'no' ? 'needsUpdate'
    : null;

  const toggle = () => {
    const opening = !open;
    setOpen(opening);
    if (!opening || localOnly || revoked || !online) return;
    setSupport('checking');
    sharedConversationDeleteSupported(assistant.ownerDeviceId).then(
      supported => setSupport(supported ? 'yes' : 'no'),
      (error) => {
        log.warn('Owner of shared assistant not asked about deleting conversations', { error: message(error) });
        setSupport('unreachable');
      },
    );
  };

  const remove = async () => {
    if (!localOnly) {
      const confirmed = await confirmDanger(
        t('nav.sessions.sharedAssistant.deleteTitle'),
        t('nav.sessions.sharedAssistant.deleteMessage', { owner }),
        { confirmText: t('nav.sessions.sharedAssistant.deleteConfirm') },
      );
      if (!confirmed) return;
    }
    setDeleting(true);
    try {
      await deleteSharedAssistantConversation(job);
    } catch (error) {
      const reason = message(explainSharedAssistantError(error, assistant.name));
      log.warn('Conversation with shared assistant not deleted', { error: message(error) });
      notificationService.error(t('nav.sessions.sharedAssistant.deleteFailed', { reason }));
    } finally {
      setDeleting(false);
    }
  };

  const items: MenuEntry[] = [{
    id: 'delete',
    label: t('nav.sessions.sharedAssistant.delete'),
    icon: <Icon name="delete" size="sm" />,
    tone: 'danger',
    disabled: block !== null,
    shortcut: block ? t(`nav.sessions.sharedAssistant.deleteBlocked.${block}`) : undefined,
    onSelect: () => void remove(),
  }];

  return (
    <div className="shared-assistant-nav__conversation-actions" data-menu-open={open || undefined}>
      <Tooltip content={moreLabel} disabled={open}>
        <IconButton
          ref={anchorRef}
          size="xs"
          variant="quiet"
          aria-label={`${moreLabel} · ${title}`}
          aria-haspopup="menu"
          aria-expanded={open}
          aria-controls={open ? menuId : undefined}
          loading={deleting}
          disabled={deleting}
          onClick={toggle}
          icon={<Icon name="more" size="xs" aria-hidden="true" />}
          data-testid="nav-shared-assistant-conversation-more"
        />
      </Tooltip>
      <MenuPopover
        id={menuId}
        aria-label={moreLabel}
        anchorRef={anchorRef}
        open={open}
        onClose={() => setOpen(false)}
        items={items}
        placement="bottom"
      />
    </div>
  );
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
  // Re-renders after an assistant's other conversations are shown.
  const [, setRevision] = useState(0);
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
      case 'stopped': return t('nav.sessions.sharedAssistant.status.stopped');
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
        const from = t('nav.sessions.sharedAssistant.from', { name: owner });
        const source = [from, standing].filter(Boolean).join(' · ');
        const key = `shared:${assistant.owner.publicKey}/${assistant.assistantId}`;
        const isActive = (sessionId: string) => isSessionNavRowActive({
          rowSessionId: sessionId,
          activeTabId,
          activeSessionId: activeSessionId ?? null,
          activeChildHasVisibleRow: false,
        });
        const statusOf = (job: ReturnType<typeof sharedAssistantConversations>[number]) => {
          // A conversation not loaded here has been sent before,
          // unless its first message never left this device.
          const turns = sessions.get(job.sessionId)?.dialogTurns?.length;
          return sharedAssistantStatus(job, turns === undefined ? job.state !== 'submitting' : turns > 0);
        };
        // Like a local assistant, the history is closed until asked for; closed, it
        // still shows the conversation being viewed and those waiting on the owner.
        const historyOpen = openedGroups.get(key) ?? false;
        const all = sharedAssistantConversations(jobs, ref);
        const { shown: conversations, hidden } = historyOpen
          ? limitRows(all, fullLists.has(key), job => isActive(job.sessionId))
          : { shown: all.filter(job => isActive(job.sessionId) || statusOf(job).kind === 'approval'), hidden: 0 };
        const historyLabel = historyOpen ? t('nav.workspaces.collapseSessions') : t('nav.sessions.history');
        // Out of reach or no longer shared: the assistant and its conversations read dimmed.
        const reachable = !revoked && assistant.online;
        return (
          <div
            key={key}
            className={['shared-assistant-nav__assistant', !reachable && 'is-unavailable'].filter(Boolean).join(' ')}
            data-testid="nav-shared-assistant"
          >
            <div className="shared-assistant-nav__header">
              {/* Who shares it reads beside the name; whether it can be reached shows as a mark, its words in the tooltip. */}
              <Tooltip content={source} placement="right" followCursor>
                <button
                  type="button"
                  className="openbitfun-nav-panel__inline-item shared-assistant-nav__name"
                  data-openbitfun-component="sessions-section"
                  data-openbitfun-part="row"
                  aria-label={`${assistant.name} · ${source}`}
                  aria-busy={busy}
                  onClick={() => run(() => openSharedAssistantConversation(ref, !revoked))}
                  data-testid="nav-shared-assistant-open"
                >
                  <span className="assistant-session-history__delegated-title">{assistant.name}</span>
                  {reachable ? (
                    <span className="shared-assistant-nav__online" data-testid="nav-shared-assistant-online" aria-hidden="true" />
                  ) : (
                    <span className="shared-assistant-nav__standing" data-testid="nav-shared-assistant-unavailable" aria-hidden="true">
                      <Icon glyph={revoked ? Ban : WifiOff} size="xs" tone="secondary" />
                    </span>
                  )}
                  <span className="shared-assistant-nav__source" data-testid="nav-shared-assistant-source" aria-hidden="true">{from}</span>
                </button>
              </Tooltip>
              <Tooltip content={historyLabel} placement="right" followCursor>
                <IconButton
                  size="xs"
                  variant="quiet"
                  className={['shared-assistant-nav__action', historyOpen && 'is-open'].filter(Boolean).join(' ')}
                  aria-label={historyLabel}
                  aria-expanded={historyOpen}
                  onClick={() => {
                    openedGroups.set(key, !historyOpen);
                    setRevision(revision => revision + 1);
                  }}
                  icon={<Icon glyph={historyOpen ? ChevronUp : History} size="xs" aria-hidden="true" />}
                  data-testid="nav-shared-assistant-history"
                />
              </Tooltip>
              {revoked ? null : (
                <Tooltip content={t('nav.sessions.sharedAssistant.newConversation')}>
                  <IconButton
                    size="xs"
                    variant="quiet"
                    className="shared-assistant-nav__action"
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
                  const status = statusOf(job);
                  const detail = statusText(status, owner);
                  const active = isActive(job.sessionId);
                  const title = sessions.get(job.sessionId)?.title || job.title || t('nav.sessions.sharedAssistant.untitled');
                  const said = detail ? `${title} · ${detail}` : title;
                  const waiting = status.kind === 'approval' || status.kind === 'queued' || status.kind === 'running';
                  return (
                    <div key={job.jobId} className="shared-assistant-nav__conversation">
                      <Tooltip content={said} placement="right" followCursor>
                        <button
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
                          aria-label={said}
                          onClick={() => run(() => openSharedAssistantConversationJob(job, ref))}
                          data-testid="nav-shared-assistant-conversation"
                        >
                          <span className="assistant-session-history__delegated-line">
                            <span className="assistant-session-history__delegated-title">{title}</span>
                            <NavTaskStatusIcon kind={status.kind} label={detail} />
                          </span>
                        </button>
                      </Tooltip>
                      <SharedConversationActions
                        job={job}
                        assistant={ref}
                        owner={owner}
                        title={title}
                        revoked={revoked}
                        online={assistant.online}
                        active={waiting}
                      />
                    </div>
                  );
                })}
                {hidden > 0 ? (
                  <button
                    type="button"
                    className="openbitfun-nav-panel__inline-item assistant-session-history__delegated-more"
                    data-openbitfun-component="sessions-section"
                    data-openbitfun-part="more"
                    onClick={() => {
                      fullLists.add(key);
                      setRevision(revision => revision + 1);
                    }}
                    data-testid="nav-shared-assistant-more"
                  >
                    {t('nav.sessions.delegated.showMore', { count: hidden })}
                  </button>
                ) : null}
              </div>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}
