import { useState, useSyncExternalStore } from 'react';
import { Icon, Tooltip } from '@openbitfun/ui';
import { useSceneStore } from '@/app/stores/sceneStore';
import { openDelegatedTask } from '@/flow_chat/session-drivers/delegated/openDelegatedTask';
import { flowChatStore } from '@/flow_chat/store/FlowChatStore';
import { isSessionNavRowActive } from '@/app/components/NavPanel/sections/sessions/sessionNavSelection';
import { createLogger } from '@/shared/utils/logger';
import { notificationService } from '@/shared/notification-system';
import type { DelegatedTask } from '@/infrastructure/api/service-api/DelegationAPI';
import { useI18n } from '@/infrastructure/i18n';
import type { WorkspaceInfo } from '@/shared/types';
import { delegatedTaskStatus, groupDelegatedTasks, limitRows, opensByDefault, type DelegatedTaskGroup } from './delegatedTaskGroups';
import { NavTaskStatusIcon } from './NavTaskStatus';
import { useAssistantDelegatedTasks } from './useAssistantDelegatedTasks';
import { fullLists, openedGroups } from './navListMemory';

const log = createLogger('DelegatedTaskList');

/**
 * Tasks other devices delegated to this assistant, listed under its own
 * sessions in one group per device. They are reviewed and approved in the
 * task's session, which opens read-only. With `waitingOnly`, only tasks that
 * wait for the owner, and the one being viewed, are listed, so they stay
 * visible while the assistant's history is collapsed. `tasks` hands in a list
 * the caller already reads; without it the list reads its own.
 */
export default function DelegatedTaskList({
  workspace,
  waitingOnly = false,
  tasks: given,
}: {
  workspace: WorkspaceInfo;
  waitingOnly?: boolean;
  tasks?: DelegatedTask[];
}) {
  const { t, formatDate } = useI18n('common');
  const read = useAssistantDelegatedTasks(workspace, given === undefined);
  const tasks = given ?? read;
  // Re-renders after a group is opened, closed, or shown in full.
  const [, setRevision] = useState(0);
  const activeTabId = useSceneStore(state => state.activeTabId);
  const activeSessionId = useSyncExternalStore(
    listener => flowChatStore.subscribe(() => listener()),
    () => flowChatStore.getState().activeSessionId,
  );

  const isActive = (task: DelegatedTask) => Boolean(task.sessionId) && isSessionNavRowActive({
    rowSessionId: task.sessionId,
    activeTabId,
    activeSessionId: activeSessionId ?? null,
    activeChildHasVisibleRow: false,
  });
  // A collapsed history still shows what waits for the owner and the task being viewed.
  const shown = waitingOnly
    ? tasks.filter(task => task.pendingPermissionCount > 0 || isActive(task))
    : tasks;
  if (shown.length === 0) return null;

  const source = (task: DelegatedTask): string => t('nav.sessions.delegated.from', {
    name: task.source.displayName || task.source.fingerprint,
    device: task.source.deviceName,
  });
  const time = (millis: number | null): string | null => (millis === null ? null
    : formatDate(new Date(millis), { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }));

  const statusText = (task: DelegatedTask): string | null => {
    switch (delegatedTaskStatus(task)) {
      case 'approval': return t('nav.sessions.delegated.waitingShort', { count: task.pendingPermissionCount });
      case 'running': return t('shared:statuses.running');
      case 'queued': return t('nav.sessions.delegated.queued');
      case 'failed': return t('shared:statuses.failed');
      default: return time(task.startedAt ? Date.parse(task.startedAt) : null);
    }
  };

  const open = (task: DelegatedTask) => {
    void openDelegatedTask(task, workspace.rootPath, workspace.id)
      .then(opened => { if (!opened) notificationService.warning(t('nav.sessions.delegated.unavailable')); })
      .catch(error => {
        log.warn('Failed to open a delegated task', { jobId: task.jobId, error });
        notificationService.warning(t('nav.sessions.delegated.unavailable'));
      });
  };

  const renderTask = (task: DelegatedTask) => {
    const kind = delegatedTaskStatus(task);
    const waiting = kind === 'approval';
    // Selected like a session row while its session is the one shown.
    const active = isActive(task);
    const label = statusText(task);
    // Collapsed, rows are not grouped, so each names its source on a second line.
    const detail = waitingOnly ? [source(task), label].filter(Boolean).join(' · ') : null;
    const title = task.title || t('nav.sessions.delegated.untitled');
    const said = [title, detail ?? label].filter(Boolean).join(' · ');
    return (
      <Tooltip key={task.jobId} content={said} placement="right" followCursor>
        <button
          type="button"
          className={['openbitfun-nav-panel__inline-item', 'assistant-session-history__delegated-item', waiting && 'is-waiting', active && 'is-active', detail && 'has-detail']
            .filter(Boolean).join(' ')}
          data-openbitfun-component="sessions-section"
          data-openbitfun-part="row"
          data-testid="nav-delegated-task"
          data-openbitfun-state={active ? 'active' : undefined}
          aria-current={active ? 'true' : undefined}
          aria-label={said}
          onClick={() => open(task)}
        >
          {detail ? (
            <>
              <span className="assistant-session-history__delegated-title">{title}</span>
              <span className="assistant-session-history__delegated-detail">{detail}</span>
            </>
          ) : (
            // Where the task stands shows as an icon; the words are in the tooltip.
            <span className="assistant-session-history__delegated-line">
              <span className="assistant-session-history__delegated-title">{title}</span>
              <NavTaskStatusIcon kind={kind} label={label} />
            </span>
          )}
        </button>
      </Tooltip>
    );
  };

  if (waitingOnly) {
    return (
      <div className="assistant-session-history__delegated" data-testid="nav-delegated-tasks">
        {shown.map(renderTask)}
      </div>
    );
  }

  const renderGroup = (group: DelegatedTaskGroup) => {
    const key = `delegated:${workspace.id}/${group.deviceId}`;
    const first = group.tasks[0];
    const isOpen = openedGroups.get(key) ?? opensByDefault(group, isActive);
    const toggle = () => {
      openedGroups.set(key, !isOpen);
      setRevision(revision => revision + 1);
    };
    const name = first.source.displayName || first.source.fingerprint;
    const detail = [
      first.source.deviceName,
      t('nav.sessions.delegated.count', { count: group.tasks.length }),
      group.running > 0 ? t('nav.sessions.delegated.running', { count: group.running }) : time(group.latestStartedAt),
    ].filter(Boolean).join(' · ');
    const { shown: rows, hidden } = limitRows(group.tasks, fullLists.has(key), isActive);
    return (
      <div
        key={group.deviceId}
        className={['assistant-session-history__delegated-group', group.waiting > 0 && 'is-waiting'].filter(Boolean).join(' ')}
        data-testid="nav-delegated-group"
      >
        <button
          type="button"
          className="assistant-session-history__delegated-heading"
          data-openbitfun-component="sessions-section"
          data-openbitfun-part="groupHeader"
          aria-expanded={isOpen}
          title={`${t('nav.sessions.delegated.fromDevice', { name })} · ${detail}`}
          onClick={toggle}
          data-testid="nav-delegated-group-toggle"
        >
          <Icon name="chevron-right" size="xs" className="assistant-session-history__delegated-chevron" aria-hidden="true" />
          <span className="assistant-session-history__delegated-device" aria-hidden="true">
            {Array.from(name)[0]?.toUpperCase() ?? '?'}
          </span>
          <span className="assistant-session-history__delegated-heading-text">
            <span className="assistant-session-history__delegated-heading-title">
              {t('nav.sessions.delegated.fromDevice', { name })}
            </span>
            <span className="assistant-session-history__delegated-heading-detail">{detail}</span>
          </span>
          {group.waiting > 0 ? (
            <span
              className="assistant-session-history__delegated-badge"
              data-openbitfun-component="sessions-section"
              data-openbitfun-part="badge"
              aria-label={t('nav.sessions.delegated.waitingBadge', { count: group.waiting })}
            >
              {group.waiting}
            </span>
          ) : null}
        </button>
        {isOpen ? (
          <>
            {rows.map(renderTask)}
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
                data-testid="nav-delegated-more"
              >
                {t('nav.sessions.delegated.showMore', { count: hidden })}
              </button>
            ) : null}
          </>
        ) : null}
      </div>
    );
  };

  return (
    <div className="assistant-session-history__delegated" data-testid="nav-delegated-tasks">
      {groupDelegatedTasks(shown).map(renderGroup)}
    </div>
  );
}
