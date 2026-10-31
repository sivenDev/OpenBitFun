import { delegationAPI, type DelegatedTask } from './DelegationAPI';
import { createLogger } from '@/shared/utils/logger';

const log = createLogger('DelegatedTaskReads');

/** How often delegated tasks are read while an assistant or its tasks are shown. */
export const DELEGATED_TASKS_REFRESH_MS = 30_000;

/** One read shared by every reader: each read starts the dispatch runner. */
let shared: { at: number; tasks: Promise<DelegatedTask[]> } | null = null;

/**
 * Reads the tasks other devices and accounts delegated to this device's
 * assistants. Readers within half a refresh period share one read; `fresh`
 * starts a new one, for example after the owner answered a prompt.
 */
export function readDelegatedTasks(options: { fresh?: boolean } = {}): Promise<DelegatedTask[]> {
  if (!options.fresh && shared && Date.now() - shared.at < DELEGATED_TASKS_REFRESH_MS / 2) {
    return shared.tasks;
  }
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
