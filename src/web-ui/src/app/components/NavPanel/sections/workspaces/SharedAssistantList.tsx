import { useId, useRef, useState, useSyncExternalStore } from 'react';
import { ActionItem, Icon, MenuPopover, OverflowText, Spinner, Tooltip, type MenuEntry } from '@openbitfun/ui';
import { Ban, ChevronUp, History, MessageCirclePlus, WifiOff } from 'lucide-react';
import { AssistantAvatar } from '@/app/components/AssistantAvatar';
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
import { NavShowMoreToggle, NavTaskRow } from './NavTaskRow';
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

interface ConversationRowProps {
  job: DispatchObserverJob;
  assistant: SharedAssistantRef;
  owner: string;
  title: string;
  status: SharedAssistantStatus;
  /** What the status says, for the tooltip and assistive technology. */
  detail: string | null;
  /** The conversation is the one being viewed. */
  selected: boolean;
  revoked: boolean;
  online: boolean;
  onOpen: () => void;
}

/**
 * One conversation with a shared assistant, laid out like a local session
 * row, with its status and then its "..." menu at the end. Deleting removes it on both devices, so the owner must be
 * reachable, still share the assistant, support it, and not be running the
 * conversation; a new conversation that never reached the owner is removed
 * here at once.
 */
function SharedConversationRow({
  job, assistant, owner, title, status, detail, selected, revoked, online, onOpen,
}: ConversationRowProps) {
  const { t } = useI18n('common');
  const [open, setOpen] = useState(false);
  const [support, setSupport] = useState<OwnerSupport>('checking');
  const [deleting, setDeleting] = useState(false);
  const anchorRef = useRef<HTMLButtonElement>(null);
  const menuId = useId();
  const localOnly = isLocalOnlyConversation(job);
  const moreLabel = t('nav.sessions.sharedAssistant.more');
  const active = status.kind === 'approval' || status.kind === 'queued' || status.kind === 'running';
  const said = detail ? `${title} · ${detail}` : title;

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
  const menuOpen = open || deleting;

  return (
    <>
      <NavTaskRow
        title={title}
        status={status.kind}
        statusLabel={detail}
        said={said}
        selected={selected}
        menuOpen={menuOpen}
        testId="nav-shared-assistant-conversation"
        onOpen={onOpen}
        actions={(
          <div
            className={`openbitfun-nav-panel__inline-item-actions${menuOpen ? ' is-open' : ''}`}
            data-openbitfun-component="sessions-section"
            data-openbitfun-part="actions"
            data-openbitfun-state={menuOpen ? 'menuOpen' : undefined}
          >
            <button
              type="button"
              ref={anchorRef}
              className={`openbitfun-nav-panel__inline-item-action-btn${menuOpen ? ' is-open' : ''}`}
              aria-label={`${title} · ${moreLabel}`}
              aria-haspopup="menu"
              aria-expanded={open}
              aria-controls={open ? menuId : undefined}
              aria-busy={deleting || undefined}
              disabled={deleting}
              onClick={(event) => {
                event.stopPropagation();
                toggle();
              }}
              data-testid="nav-shared-assistant-conversation-more"
            >
              {deleting ? <Spinner size="xs" /> : <Icon name="more" size="xs" />}
            </button>
          </div>
        )}
      />
      <MenuPopover
        id={menuId}
        aria-label={moreLabel}
        anchorRef={anchorRef}
        open={open}
        onClose={() => setOpen(false)}
        items={items}
        placement="bottom"
      />
    </>
  );
}

/**
 * Assistants other devices share with this one, each with its conversations.
 * They run on their owner's device and belong to no workspace here, so they
 * have their own group, but each reads like a local assistant: its card, then
 * its conversations as session rows. A connected peer's list is not this
 * device's, so nothing is listed in Peer Device Mode.
 */
export default function SharedAssistantList() {
  const { t } = useI18n('common');
  const peerActive = usePeerDeviceModeOptional()?.peerMode?.active === true;
  const sharedWithMe = useLanShareStore(state => state.sharedWithMe);
  const jobs = dispatchJobStore(state => state.jobs);
  const [busy, setBusy] = useState(false);
  // Re-renders after an assistant's history is opened, closed, or shown in full.
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
        const statusOf = (job: DispatchObserverJob) => {
          // A conversation not loaded here has been sent before,
          // unless its first message never left this device.
          const turns = sessions.get(job.sessionId)?.dialogTurns?.length;
          return sharedAssistantStatus(job, turns === undefined ? job.state !== 'submitting' : turns > 0);
        };
        // Like a local assistant, the history is closed until asked for; closed, it
        // still shows the conversation being viewed and those waiting on the owner.
        const historyOpen = openedGroups.get(key) ?? false;
        const all = sharedAssistantConversations(jobs, ref);
        const showAll = fullLists.has(key);
        const limited = limitRows(all, false, job => isActive(job.sessionId));
        const conversations = historyOpen
          ? (showAll ? all : limited.shown)
          : all.filter(job => isActive(job.sessionId) || statusOf(job).kind === 'approval');
        const historyLabel = historyOpen ? t('nav.workspaces.collapseSessions') : t('nav.sessions.history');
        // Out of reach or no longer shared: the assistant and its conversations read dimmed.
        const reachable = !revoked && assistant.online;
        const viewing = all.some(job => isActive(job.sessionId));
        const open = () => run(() => openSharedAssistantConversation(ref, !revoked));
        const toggleFull = () => {
          if (showAll) fullLists.delete(key);
          else fullLists.add(key);
          setRevision(revision => revision + 1);
        };
        return (
          <div
            key={key}
            className={[
              'openbitfun-nav-panel__assistant-item',
              'shared-assistant-nav__assistant',
              viewing && 'is-active',
              !reachable && 'is-unavailable',
            ].filter(Boolean).join(' ')}
            data-testid="nav-shared-assistant"
          >
            <div className="openbitfun-nav-panel__assistant-item-card" data-openbitfun-product-component="workspace-item" data-openbitfun-product-part="card">
              <button
                type="button"
                className="openbitfun-nav-panel__assistant-item-open-btn"
                data-openbitfun-product-component="workspace-item"
                data-openbitfun-product-part="action"
                tabIndex={-1}
                aria-hidden="true"
                onClick={open}
              >
                <span className="openbitfun-nav-panel__assistant-item-avatar" data-openbitfun-product-component="workspace-item" data-openbitfun-product-part="icon">
                  <AssistantAvatar size={22} name={assistant.name} />
                </span>
              </button>
              {/* Who shares it reads beside the name; whether it can be reached shows as a mark, its words in the tooltip. */}
              <Tooltip content={source} placement="right" followCursor>
                <ActionItem
                  data-overflow-trigger
                  data-openbitfun-product-component="workspace-item"
                  data-openbitfun-product-part="name"
                  type="button"
                  className="openbitfun-nav-panel__assistant-item-name-action"
                  triggerClassName="openbitfun-nav-panel__assistant-item-name-btn shared-assistant-nav__name"
                  labelBehavior="static"
                  aria-label={`${assistant.name} · ${source}`}
                  aria-busy={busy}
                  onClick={open}
                  data-testid="nav-shared-assistant-open"
                >
                  <OverflowText className="openbitfun-nav-panel__assistant-item-label">{assistant.name}</OverflowText>
                  {reachable ? (
                    <span className="shared-assistant-nav__online" data-testid="nav-shared-assistant-online" aria-hidden="true" />
                  ) : (
                    <span className="shared-assistant-nav__standing" data-testid="nav-shared-assistant-unavailable" aria-hidden="true">
                      <Icon glyph={revoked ? Ban : WifiOff} size="xs" tone="secondary" />
                    </span>
                  )}
                  <span className="shared-assistant-nav__source" data-testid="nav-shared-assistant-source" aria-hidden="true">{from}</span>
                </ActionItem>
              </Tooltip>
              <div className="openbitfun-nav-panel__assistant-item-menu" data-openbitfun-product-component="workspace-item" data-openbitfun-product-part="menu">
                <Tooltip content={historyLabel} placement="right" followCursor>
                  <button
                    type="button"
                    className={`openbitfun-nav-panel__assistant-item-menu-trigger${historyOpen ? ' is-open' : ''}`}
                    data-openbitfun-product-component="workspace-item"
                    data-openbitfun-product-part="action"
                    aria-label={historyLabel}
                    aria-expanded={historyOpen}
                    onClick={() => {
                      openedGroups.set(key, !historyOpen);
                      setRevision(revision => revision + 1);
                    }}
                    data-testid="nav-shared-assistant-history"
                  >
                    <Icon glyph={historyOpen ? ChevronUp : History} size="xs" />
                  </button>
                </Tooltip>
                {revoked ? null : (
                  <Tooltip content={t('nav.sessions.sharedAssistant.newConversation')} placement="right" followCursor>
                    <button
                      type="button"
                      className="openbitfun-nav-panel__assistant-item-menu-trigger"
                      data-openbitfun-product-component="workspace-item"
                      data-openbitfun-product-part="action"
                      aria-label={t('nav.sessions.sharedAssistant.newConversation')}
                      disabled={busy}
                      onClick={() => run(() => startSharedAssistantConversation(ref))}
                      data-testid="nav-shared-assistant-new"
                    >
                      <Icon glyph={MessageCirclePlus} size="xs" />
                    </button>
                  </Tooltip>
                )}
              </div>
            </div>
            {conversations.length > 0 ? (
              <div className="openbitfun-nav-panel__workspace-item-sessions">
                <div className="assistant-session-history" data-openbitfun-component="sessions-section" data-openbitfun-part="root">
                  <div className="openbitfun-nav-panel__inline-list">
                    {conversations.map((job) => {
                      const status = statusOf(job);
                      const title = sessions.get(job.sessionId)?.title || job.title || t('nav.sessions.sharedAssistant.untitled');
                      return (
                        <SharedConversationRow
                          key={job.jobId}
                          job={job}
                          assistant={ref}
                          owner={owner}
                          title={title}
                          status={status}
                          detail={statusText(status, owner)}
                          selected={isActive(job.sessionId)}
                          revoked={revoked}
                          online={assistant.online}
                          onOpen={() => run(() => openSharedAssistantConversationJob(job, ref))}
                        />
                      );
                    })}
                    {historyOpen && limited.hidden > 0 ? (
                      <NavShowMoreToggle hidden={limited.hidden} showAll={showAll} onToggle={toggleFull} testId="nav-shared-assistant-more" />
                    ) : null}
                  </div>
                </div>
              </div>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}
