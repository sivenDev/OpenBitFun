import { useEffect, useState } from 'react';
import { useSceneStore } from '@/app/stores/sceneStore';
import type { DelegatedTask } from '@/infrastructure/api/service-api/DelegationAPI';
import { useI18n } from '@/infrastructure/i18n';
import { usePeerDeviceModeOptional } from '@/infrastructure/peer-device/peerDeviceContextState';
import { isTauriRuntime } from '@/infrastructure/runtime';
import type { WorkspaceInfo } from '@/shared/types';
import { DELEGATED_TASKS_REFRESH_MS, readDelegatedTasks } from './delegatedTaskReads';

function samePath(left: string | undefined, right: string): boolean {
  const trim = (path: string) => path.replace(/[\\/]+$/, '');
  return left !== undefined && trim(left) === trim(right);
}

/**
 * Tasks other devices and accounts delegated to this assistant, listed under
 * its own sessions with their source. They are reviewed and approved on the
 * AI collaboration page. A connected peer's assistants are not this device's,
 * so nothing is listed in Peer Device Mode.
 */
export default function DelegatedTaskList({ workspace }: { workspace: WorkspaceInfo }) {
  const { t } = useI18n('common');
  const peerActive = usePeerDeviceModeOptional()?.peerMode?.active === true;
  const enabled = isTauriRuntime() && !peerActive && !workspace.connectionId;
  const [tasks, setTasks] = useState<DelegatedTask[]>([]);

  useEffect(() => {
    if (!enabled) {
      setTasks([]);
      return undefined;
    }
    let active = true;
    const refresh = () => {
      void readDelegatedTasks().then(all => {
        if (active) setTasks(all.filter(task => samePath(task.assistantWorkspace, workspace.rootPath)));
      });
    };
    refresh();
    const timer = setInterval(refresh, DELEGATED_TASKS_REFRESH_MS);
    return () => {
      active = false;
      clearInterval(timer);
    };
  }, [enabled, workspace.rootPath]);

  if (tasks.length === 0) return null;

  const source = (task: DelegatedTask): string => {
    if (task.source.kind === 'connection') {
      return t('nav.sessions.delegated.fromAccount', {
        name: task.source.accountName ?? task.source.connectionId ?? '',
      });
    }
    return t('nav.sessions.delegated.fromDevice', {
      device: task.source.deviceName ?? task.source.deviceId ?? '',
    });
  };

  return (
    <div
      className="assistant-session-history__delegated"
      data-openbitfun-component="sessions-section"
      data-openbitfun-part="delegated-tasks"
      data-testid="nav-delegated-tasks"
    >
      {tasks.map(task => {
        const waiting = task.pendingPermissionCount > 0;
        const detail = [
          t('nav.sessions.delegated.external'),
          source(task),
          waiting ? t('nav.sessions.delegated.waiting', { count: task.pendingPermissionCount }) : null,
        ].filter(Boolean).join(' · ');
        return (
          <button
            key={task.jobId}
            type="button"
            className={['openbitfun-nav-panel__inline-item', 'assistant-session-history__delegated-item', waiting && 'is-waiting']
              .filter(Boolean).join(' ')}
            data-openbitfun-component="sessions-section"
            data-openbitfun-part="delegated-task"
            data-openbitfun-state={waiting ? 'attention' : undefined}
            data-testid="nav-delegated-task"
            title={detail}
            onClick={() => useSceneStore.getState().openScene('ai-collaboration')}
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
