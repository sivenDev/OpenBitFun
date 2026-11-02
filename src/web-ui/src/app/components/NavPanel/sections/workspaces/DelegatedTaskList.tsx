import { useEffect, useState, useSyncExternalStore } from 'react';
import { useSceneStore } from '@/app/stores/sceneStore';
import { openDelegatedTask } from '@/flow_chat/session-drivers/delegated/openDelegatedTask';
import { flowChatStore } from '@/flow_chat/store/FlowChatStore';
import { isSessionNavRowActive } from '@/app/components/NavPanel/sections/sessions/sessionNavSelection';
import { createLogger } from '@/shared/utils/logger';
import { notificationService } from '@/shared/notification-system';
import { ASSISTANT_AGENT_TYPE_PREFIX, type DelegatedTask } from '@/infrastructure/api/service-api/DelegationAPI';
import { useI18n } from '@/infrastructure/i18n';
import { usePeerDeviceModeOptional } from '@/infrastructure/peer-device/peerDeviceContextState';
import { isTauriRuntime } from '@/infrastructure/runtime';
import type { WorkspaceInfo } from '@/shared/types';
import {
  DELEGATED_TASKS_REFRESH_MS,
  onDelegatedTasksRead,
  readDelegatedTasks,
} from '@/infrastructure/api/service-api/delegatedTaskReads';

const log = createLogger('DelegatedTaskList');

/**
 * Tasks name the assistant they run as by its directory (`assistant:<dir>`).
 * Paths are not compared: the runner and the app may spell the same directory
 * differently, for example through a symlinked temporary or home directory.
 */
function runsAs(task: DelegatedTask, rootPath: string): boolean {
  const directory = rootPath.replace(/[\\/]+$/, '').split(/[\\/]/).pop() ?? '';
  return directory !== '' && task.agentType === `${ASSISTANT_AGENT_TYPE_PREFIX}${directory}`;
}

/**
 * Tasks other devices and accounts delegated to this assistant, listed under
 * its own sessions with their source. They are reviewed and approved in the
 * task's session, which opens read-only.
 * A connected peer's assistants are not this device's,
 * so nothing is listed in Peer Device Mode. With `waitingOnly`, only tasks
 * that wait for the owner, and the one being viewed, are listed, so they
 * stay visible while the assistant's history is collapsed.
 */
export default function DelegatedTaskList({
  workspace,
  waitingOnly = false,
}: {
  workspace: WorkspaceInfo;
  waitingOnly?: boolean;
}) {
  const { t } = useI18n('common');
  const peerActive = usePeerDeviceModeOptional()?.peerMode?.active === true;
  const enabled = isTauriRuntime() && !peerActive && !workspace.connectionId;
  const [tasks, setTasks] = useState<DelegatedTask[]>([]);
  const activeTabId = useSceneStore(state => state.activeTabId);
  const activeSessionId = useSyncExternalStore(
    listener => flowChatStore.subscribe(() => listener()),
    () => flowChatStore.getState().activeSessionId,
  );

  useEffect(() => {
    if (!enabled) {
      setTasks([]);
      return undefined;
    }
    let active = true;
    const show = (all: DelegatedTask[]) => {
      if (active) setTasks(all.filter(task => runsAs(task, workspace.rootPath)));
    };
    const refresh = () => { void readDelegatedTasks().then(show); };
    refresh();
    // Reads started elsewhere, such as the AI collaboration page after an answer, show here too.
    const unsubscribe = onDelegatedTasksRead(show);
    const timer = setInterval(refresh, DELEGATED_TASKS_REFRESH_MS);
    return () => {
      active = false;
      unsubscribe();
      clearInterval(timer);
    };
  }, [enabled, workspace.rootPath]);

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

  return (
    <div
      className="assistant-session-history__delegated"
      data-testid="nav-delegated-tasks"
    >
      {shown.map(task => {
        const waiting = task.pendingPermissionCount > 0;
        // Selected like a session row while its session is the one shown.
        const active = isActive(task);
        const detail = [
          t('nav.sessions.delegated.external'),
          source(task),
          waiting ? t('nav.sessions.delegated.waiting', { count: task.pendingPermissionCount }) : null,
        ].filter(Boolean).join(' · ');
        return (
          <button
            key={task.jobId}
            type="button"
            className={['openbitfun-nav-panel__inline-item', 'assistant-session-history__delegated-item', waiting && 'is-waiting', active && 'is-active']
              .filter(Boolean).join(' ')}
            data-openbitfun-component="sessions-section"
            data-openbitfun-part="row"
            data-testid="nav-delegated-task"
            data-openbitfun-state={active ? 'active' : undefined}
            aria-current={active ? 'true' : undefined}
            title={detail}
            onClick={() => {
              void openDelegatedTask(task, workspace.rootPath, workspace.id)
                .then(opened => { if (!opened) notificationService.warning(t('nav.sessions.delegated.unavailable')); })
                .catch(error => {
                  log.warn('Failed to open a delegated task', { jobId: task.jobId, error });
                  notificationService.warning(t('nav.sessions.delegated.unavailable'));
                });
            }}
          >
            <span className="assistant-session-history__delegated-title">
              {task.title || t('nav.sessions.delegated.untitled')}
            </span>
            <span className="assistant-session-history__delegated-detail">{detail}</span>
          </button>
        );
      })}
    </div>
  );
}
