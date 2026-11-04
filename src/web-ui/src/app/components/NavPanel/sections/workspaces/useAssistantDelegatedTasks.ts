import { useEffect, useState } from 'react';
import { ASSISTANT_AGENT_TYPE_PREFIX, type DelegatedTask } from '@/infrastructure/api/service-api/DelegationAPI';
import { usePeerDeviceModeOptional } from '@/infrastructure/peer-device/peerDeviceContextState';
import { isTauriRuntime } from '@/infrastructure/runtime';
import type { WorkspaceInfo } from '@/shared/types';
import {
  DELEGATED_TASKS_REFRESH_MS,
  onDelegatedTasksRead,
  readDelegatedTasks,
} from '@/infrastructure/api/service-api/delegatedTaskReads';

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
 * Tasks other devices gave this assistant, kept fresh. A connected peer's
 * assistants are not this device's, so none are read in Peer Device Mode;
 * with `enabled` false nothing is read, for callers handed the list instead.
 */
export function useAssistantDelegatedTasks(workspace: WorkspaceInfo, enabled = true): DelegatedTask[] {
  const peerActive = usePeerDeviceModeOptional()?.peerMode?.active === true;
  const reads = enabled && isTauriRuntime() && !peerActive && !workspace.connectionId;
  const [tasks, setTasks] = useState<DelegatedTask[]>([]);

  useEffect(() => {
    if (!reads) {
      setTasks([]);
      return undefined;
    }
    let active = true;
    const show = (all: DelegatedTask[]) => {
      if (active) setTasks(all.filter(task => runsAs(task, workspace.rootPath)));
    };
    const refresh = () => { void readDelegatedTasks().then(show); };
    refresh();
    // Reads started elsewhere, such as after an answer, show here too.
    const unsubscribe = onDelegatedTasksRead(show);
    const timer = setInterval(refresh, DELEGATED_TASKS_REFRESH_MS);
    return () => {
      active = false;
      unsubscribe();
      clearInterval(timer);
    };
  }, [reads, workspace.rootPath]);

  return tasks;
}
