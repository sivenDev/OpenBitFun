import { describe, expect, it } from 'vitest';
import type { DelegatedTask } from '@/infrastructure/api/service-api/DelegationAPI';
import { delegatedTaskStatus, groupDelegatedTasks, limitRows, opensByDefault } from './delegatedTaskGroups';

function task(jobId: string, overrides: Partial<DelegatedTask> = {}, deviceId = 'ann'): DelegatedTask {
  return {
    jobId, sessionId: `s-${jobId}`, state: 'succeeded', title: jobId, assistantId: 'workspace',
    agentType: 'assistant:workspace', pendingPermissionCount: 0,
    source: { deviceId, fingerprint: 'AB23CD', displayName: deviceId, deviceName: `${deviceId}-mac` },
    ...overrides,
  };
}

describe('delegatedTaskGroups', () => {
  it('reads a task as waiting before anything else, then by its state', () => {
    expect(delegatedTaskStatus(task('a', { state: 'running', pendingPermissionCount: 1 }))).toBe('approval');
    expect(delegatedTaskStatus(task('a', { state: 'running' }))).toBe('running');
    expect(delegatedTaskStatus(task('a', { state: 'queued' }))).toBe('queued');
    expect(delegatedTaskStatus(task('a', { state: 'failed' }))).toBe('failed');
    expect(delegatedTaskStatus(task('a', { state: 'cancelled' }))).toBe('done');
  });

  it('puts the device waiting for the owner first, and its tasks waiting, running, then newest', () => {
    const groups = groupDelegatedTasks([
      task('old', { startedAt: '2026-11-03T01:00:00Z' }),
      task('new', { startedAt: '2026-11-03T03:00:00Z' }),
      task('busy', { state: 'running', startedAt: '2026-11-03T00:00:00Z' }),
      task('bea-late', { startedAt: '2026-11-03T05:00:00Z' }, 'bea'),
      task('cy-asks', { pendingPermissionCount: 2, startedAt: '2026-11-02T00:00:00Z' }, 'cy'),
      task('cy-asks-too', { pendingPermissionCount: 1 }, 'cy'),
    ]);
    expect(groups.map(group => group.deviceId)).toEqual(['cy', 'bea', 'ann']);
    expect(groups[0]).toMatchObject({ waiting: 3, running: 0 });
    expect(groups[2].tasks.map(entry => entry.jobId)).toEqual(['busy', 'new', 'old']);
    expect(groups[2]).toMatchObject({ running: 1, latestStartedAt: Date.parse('2026-11-03T03:00:00Z') });
  });

  it('opens a group that waits or holds the viewed session, and keeps the others closed', () => {
    const [waiting] = groupDelegatedTasks([task('a', { pendingPermissionCount: 1 })]);
    const [quiet] = groupDelegatedTasks([task('b'), task('c')]);
    expect(opensByDefault(waiting, () => false)).toBe(true);
    expect(opensByDefault(quiet, () => false)).toBe(false);
    expect(opensByDefault(quiet, entry => entry.jobId === 'c')).toBe(true);
  });

  it('shows the first rows and the viewed one, and counts the rest', () => {
    const items = ['a', 'b', 'c', 'd', 'e', 'f'];
    expect(limitRows(items, false, () => false)).toEqual({ shown: ['a', 'b', 'c'], hidden: 3 });
    expect(limitRows(items, false, item => item === 'e')).toEqual({ shown: ['a', 'b', 'c', 'e'], hidden: 2 });
    expect(limitRows(items, true, () => false)).toEqual({ shown: items, hidden: 0 });
    expect(limitRows(['a', 'b'], false, () => false)).toEqual({ shown: ['a', 'b'], hidden: 0 });
  });
});
