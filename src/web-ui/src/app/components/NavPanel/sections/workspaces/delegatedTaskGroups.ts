import type { DelegatedTask } from '@/infrastructure/api/service-api/DelegationAPI';
import type { NavTaskStatusKind } from './NavTaskStatus';

/** Rows a closed-up group shows before "show the other N". */
export const GROUP_ROW_LIMIT = 3;

/** The tasks one device gave an assistant, as its owner sees them. */
export interface DelegatedTaskGroup {
  deviceId: string;
  /** Waiting first, then running, then the newest started. */
  tasks: DelegatedTask[];
  /** Actions waiting for the owner, over all the group's tasks. */
  waiting: number;
  running: number;
  /** Milliseconds; null when no task has a start time. */
  latestStartedAt: number | null;
}

export function delegatedTaskStatus(task: DelegatedTask): NavTaskStatusKind {
  if (task.pendingPermissionCount > 0) return 'approval';
  switch (task.state) {
    case 'queued': return 'queued';
    case 'submitting':
    case 'running': return 'running';
    case 'failed': return 'failed';
    default: return 'done';
  }
}

function startedAt(task: DelegatedTask): number | null {
  const time = task.startedAt ? Date.parse(task.startedAt) : Number.NaN;
  return Number.isFinite(time) ? time : null;
}

function rank(task: DelegatedTask): number {
  switch (delegatedTaskStatus(task)) {
    case 'approval': return 0;
    case 'running':
    case 'queued': return 1;
    default: return 2;
  }
}

/** Newer first; tasks without a start time keep their place after those with one. */
function byStart(a: DelegatedTask, b: DelegatedTask): number {
  const left = startedAt(a);
  const right = startedAt(b);
  if (left === right) return 0;
  if (left === null) return 1;
  if (right === null) return -1;
  return right - left;
}

/**
 * One group per device that sent tasks. Groups with something waiting for
 * the owner come first, the rest by their newest task; the sort is stable,
 * so equal groups and tasks keep the order the list came in.
 */
export function groupDelegatedTasks(tasks: DelegatedTask[]): DelegatedTaskGroup[] {
  const byDevice = new Map<string, DelegatedTask[]>();
  for (const task of tasks) {
    const group = byDevice.get(task.source.deviceId);
    if (group) group.push(task);
    else byDevice.set(task.source.deviceId, [task]);
  }
  const groups = Array.from(byDevice, ([deviceId, members]): DelegatedTaskGroup => {
    const times = members.map(startedAt).filter((time): time is number => time !== null);
    return {
      deviceId,
      tasks: [...members].sort((a, b) => rank(a) - rank(b) || byStart(a, b)),
      waiting: members.reduce((sum, task) => sum + task.pendingPermissionCount, 0),
      running: members.filter(task => delegatedTaskStatus(task) === 'running').length,
      latestStartedAt: times.length > 0 ? Math.max(...times) : null,
    };
  });
  return groups.sort((a, b) => {
    if ((a.waiting > 0) !== (b.waiting > 0)) return a.waiting > 0 ? -1 : 1;
    return (b.latestStartedAt ?? -Infinity) - (a.latestStartedAt ?? -Infinity) || 0;
  });
}

/**
 * A group opens by itself while it waits for the owner or holds the
 * session being viewed; otherwise it stays closed until opened.
 */
export function opensByDefault(group: DelegatedTaskGroup, isActive: (task: DelegatedTask) => boolean): boolean {
  return group.waiting > 0 || group.tasks.some(isActive);
}

/**
 * The first rows of a list, plus the one being viewed when it falls past
 * them, and how many stay hidden.
 */
export function limitRows<T>(items: T[], showAll: boolean, isActive: (item: T) => boolean, limit = GROUP_ROW_LIMIT): { shown: T[]; hidden: number } {
  if (showAll || items.length <= limit) return { shown: items, hidden: 0 };
  const shown = items.slice(0, limit);
  const active = items.slice(limit).find(isActive);
  if (active) shown.push(active);
  return { shown, hidden: items.length - shown.length };
}
