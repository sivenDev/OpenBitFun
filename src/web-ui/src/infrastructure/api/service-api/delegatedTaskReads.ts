import { delegationAPI, type DelegatedTask } from './DelegationAPI';
import { createLogger } from '@/shared/utils/logger';

const log = createLogger('DelegatedTaskReads');

/** How often delegated tasks are read while an assistant or its tasks are shown. */
export const DELEGATED_TASKS_REFRESH_MS = 30_000;

/** One read shared by every reader: each read starts the dispatch runner. */
let shared: { at: number; tasks: Promise<DelegatedTask[]> } | null = null;

/**
 * Readers told about every new read, so one surface's refresh reaches the
 * others. `listed` is false when the read failed and `tasks` is empty for it.
 */
type DelegatedTasksListener = (tasks: DelegatedTask[], listed: boolean) => void;
const listeners = new Set<DelegatedTasksListener>();

/**
 * Reads the tasks other devices and accounts delegated to this device's
 * assistants. Readers within half a refresh period share one read; `fresh`
 * starts a new one, for example after the owner answered a prompt.
 */
export function readDelegatedTasks(options: { fresh?: boolean } = {}): Promise<DelegatedTask[]> {
  if (!options.fresh && shared && Date.now() - shared.at < DELEGATED_TASKS_REFRESH_MS / 2) {
    return shared.tasks;
  }
  const read = delegationAPI.listTasks().then(
    listed => ({ listed: true, tasks: listed }),
    (error: unknown) => {
      log.warn('Failed to list delegated tasks', { error });
      return { listed: false, tasks: [] as DelegatedTask[] };
    },
  );
  const tasks = read.then(result => result.tasks);
  shared = { at: Date.now(), tasks };
  void read.then(result => {
    if (shared?.tasks === tasks) listeners.forEach(listener => listener(result.tasks, result.listed));
  });
  return tasks;
}

/** Calls `listener` with the result of every new read; returns the unsubscribe. */
export function onDelegatedTasksRead(listener: DelegatedTasksListener): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

/** Test seam: forget the shared read. */
export function resetDelegatedTaskReads(): void {
  shared = null;
  listeners.clear();
}
