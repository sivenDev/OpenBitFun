import type { DelegatedTask } from '@/infrastructure/api/service-api/DelegationAPI';
import { workspaceManager } from '@/infrastructure/services/business/workspaceManager';
import { openMainSession } from '../../services/sessionActivation';
import { followDelegatedTask } from './delegatedTaskFollower';

const comparablePath = (path: string) => path.replace(/^\/private(?=\/)/, '').replace(/[\\/]+$/, '');

/**
 * Open a delegated task's session beside its assistant's own, read-only.
 * `workspaceId` is the assistant's open workspace when the caller knows it.
 */
export async function openDelegatedTask(
  task: DelegatedTask,
  assistantRoot: string,
  workspaceId?: string,
): Promise<boolean> {
  const workspace = workspaceId
    ? undefined
    : Array.from(workspaceManager.getState().openedWorkspaces.values())
      .find(candidate => comparablePath(candidate.rootPath) === comparablePath(assistantRoot));
  const followed = followDelegatedTask({
    jobId: task.jobId,
    sessionId: task.sessionId,
    title: task.title,
    state: task.state,
    workspacePath: assistantRoot,
    workspaceId: workspaceId ?? workspace?.id,
  });
  if (!followed) return false;
  await openMainSession(task.sessionId, { workspaceId: workspaceId ?? workspace?.id });
  return true;
}
