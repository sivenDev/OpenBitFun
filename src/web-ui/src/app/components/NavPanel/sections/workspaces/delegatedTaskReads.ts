import { delegationAPI, type DelegatedTask } from '@/infrastructure/api/service-api/DelegationAPI';
import { createLogger } from '@/shared/utils/logger';

const log = createLogger('DelegatedTaskList');

/** How often the list of delegated tasks is read while an assistant is shown. */
export const DELEGATED_TASKS_REFRESH_MS = 30_000;

/** One read shared by every assistant row: each read starts the dispatch runner. */
let shared: { at: number; tasks: Promise<DelegatedTask[]> } | null = null;

export function readDelegatedTasks(): Promise<DelegatedTask[]> {
  if (shared && Date.now() - shared.at < DELEGATED_TASKS_REFRESH_MS / 2) return shared.tasks;
  const tasks = delegationAPI.listTasks().catch((error: unknown) => {
    log.warn('Failed to list delegated tasks', { error });
    return [] as DelegatedTask[];
  });
  shared = { at: Date.now(), tasks };
  return tasks;
}

/** Test seam: forget the shared read. */
export function resetDelegatedTaskReads(): void {
  shared = null;
}
