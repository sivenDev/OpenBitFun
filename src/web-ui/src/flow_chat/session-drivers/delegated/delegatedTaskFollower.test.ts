import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ listTasks: vi.fn(), taskEvents: vi.fn(), warning: vi.fn() }));

vi.mock('@/infrastructure/api/service-api/DelegationAPI', () => ({
  delegationAPI: { listTasks: mocks.listTasks, taskEvents: mocks.taskEvents },
}));
vi.mock('@/infrastructure/i18n', () => ({ i18nService: { t: (key: string) => key } }));
vi.mock('@/shared/notification-system', () => ({ notificationService: { warning: mocks.warning } }));
vi.mock('../../services/AgenticEventListener', () => ({ agenticEventListener: { dispatchExternal: vi.fn() } }));
vi.mock('@/features/dispatch/DispatchJobObserver', () => ({
  dispatchEventId: () => 'event',
  projectDispatchAgentEvent: () => null,
}));

import { readDelegatedTasks, resetDelegatedTaskReads } from '@/infrastructure/api/service-api/delegatedTaskReads';
import type { FlowChatContext } from '../../services/flow-chat-manager/types';
import { delegatedSessionStore } from './delegatedSessionStore';
import { followDelegatedTask, forgetDelegatedTask, installDelegatedTaskFollower } from './delegatedTaskFollower';

const sessions = new Map<string, unknown>();
const context = {
  flowChatStore: {
    getState: () => ({ sessions }),
    addExternalSession: (sessionId: string) => { sessions.set(sessionId, {}); },
  },
  eventBatcher: { flushNow: vi.fn() },
} as unknown as FlowChatContext;

const settle = () => new Promise(resolve => setTimeout(resolve, 0));

describe('the view of a delegated task', () => {
  let uninstall: () => void;
  const closeView = vi.fn(async (sessionId: string) => {
    sessions.delete(sessionId);
    forgetDelegatedTask(sessionId);
  });

  beforeEach(() => {
    resetDelegatedTaskReads();
    sessions.clear();
    closeView.mockClear();
    mocks.warning.mockReset();
    // A finished task whose log is fully replayed: the follower stops reading.
    mocks.taskEvents.mockResolvedValue({ events: [], cursor: 0, state: 'succeeded', pendingPermissions: [] });
    uninstall = installDelegatedTaskFollower(context, closeView);
    followDelegatedTask({ jobId: 'job-1', sessionId: 'session-1', title: 'Hi', state: 'succeeded', workspacePath: '/assistant' });
  });

  afterEach(() => {
    uninstall();
    delegatedSessionStore.getState().remove('session-1');
  });

  it('stays open while the runner lists its task, and when the list cannot be read', async () => {
    mocks.listTasks.mockResolvedValueOnce([{ jobId: 'job-1' }]);
    await readDelegatedTasks({ fresh: true });
    mocks.listTasks.mockRejectedValueOnce(new Error('runner unavailable'));
    await readDelegatedTasks({ fresh: true });
    await settle();
    expect(closeView).not.toHaveBeenCalled();
    expect(sessions.has('session-1')).toBe(true);
  });

  it('closes once and says so when the device that sent the task deleted it', async () => {
    mocks.listTasks.mockResolvedValue([{ jobId: 'job-2' }]);
    await readDelegatedTasks({ fresh: true });
    await readDelegatedTasks({ fresh: true });
    await settle();
    expect(closeView).toHaveBeenCalledTimes(1);
    expect(closeView).toHaveBeenCalledWith('session-1');
    expect(mocks.warning).toHaveBeenCalledWith('common:nav.sessions.delegated.unavailable');
  });
});
