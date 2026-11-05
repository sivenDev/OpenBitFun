import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { DispatchObserverJob } from './dispatchJobStore';

const mocks = vi.hoisted(() => ({
  jobs: {} as Record<string, unknown>,
  create: vi.fn(),
  remove: vi.fn(),
  open: vi.fn(),
  removeJob: vi.fn(),
  dismissJob: vi.fn(),
  support: vi.fn(),
  deleteConversation: vi.fn(),
  sessions: new Map<string, unknown>(),
}));
vi.mock('@/flow_chat/services/FlowChatManager', () => ({
  flowChatManager: {
    createSharedAssistantSession: mocks.create,
    deleteChatSession: mocks.remove,
  },
}));
vi.mock('@/flow_chat/services/sessionActivation', () => ({ openMainSession: mocks.open }));
vi.mock('./dispatchJobStore', () => ({
  dispatchJobStore: { getState: () => ({ jobs: mocks.jobs, removeJob: mocks.removeJob, dismissJob: mocks.dismissJob }) },
}));
vi.mock('@/infrastructure/api/service-api/LanShareAPI', () => ({
  lanShareAPI: { conversationSupport: mocks.support, deleteConversation: mocks.deleteConversation },
}));
vi.mock('@/flow_chat/store/FlowChatStore', () => ({
  flowChatStore: { getState: () => ({ sessions: mocks.sessions }) },
}));

import {
  deleteSharedAssistantConversation,
  forgetSharedAssistantConversations,
  isLocalOnlyConversation,
  openSharedAssistantConversation,
  openSharedAssistantConversationJob,
  resetSharedConversationSupportForTest,
  sharedAssistantConversations,
  sharedConversationDeleteSupported,
  startSharedAssistantConversation,
} from './sharedAssistantConversation';

const writer = { ownerDeviceId: 'ann-device', assistantId: 'workspace', name: 'Writer' };

function job(jobId: string, updatedAt: number, target: DispatchObserverJob['target']): DispatchObserverJob {
  return { jobId, sessionId: `session-${jobId}`, target, updatedAt } as DispatchObserverJob;
}
const withWriter = (jobId: string, updatedAt: number) => job(jobId, updatedAt, {
  kind: 'sharedAssistant',
  ownerDeviceId: 'ann-device',
  assistantId: 'workspace',
  displayName: 'Writer',
});

beforeEach(() => {
  for (const mock of [mocks.create, mocks.remove, mocks.open, mocks.removeJob, mocks.dismissJob, mocks.support, mocks.deleteConversation]) {
    mock.mockReset();
  }
  resetSharedConversationSupportForTest();
  mocks.sessions = new Map();
  mocks.create.mockResolvedValue('session-new');
  mocks.jobs = {
    old: withWriter('old', 1),
    latest: withWriter('latest', 3),
    other: job('other', 5, { kind: 'sharedAssistant', ownerDeviceId: 'ann-device', assistantId: 'coder', displayName: 'Coder' }),
    elsewhere: job('elsewhere', 9, { kind: 'device', deviceId: 'ann-device', displayName: 'Ann' } as DispatchObserverJob['target']),
  };
});

describe('conversations with a shared assistant', () => {
  it('are the jobs for that assistant of that device, most recent first', () => {
    const found = sharedAssistantConversations(mocks.jobs as Record<string, DispatchObserverJob>, writer);
    expect(found.map(({ jobId }) => jobId)).toEqual(['latest', 'old']);
  });

  it('open at the latest one', async () => {
    await expect(openSharedAssistantConversation(writer, true)).resolves.toBe('session-latest');
    expect(mocks.open).toHaveBeenCalledWith('session-latest');
    expect(mocks.create).not.toHaveBeenCalled();
  });

  it('start one when there is none, unless none may start', async () => {
    mocks.jobs = {};
    await expect(openSharedAssistantConversation(writer, false)).resolves.toBeUndefined();
    expect(mocks.create).not.toHaveBeenCalled();

    await expect(openSharedAssistantConversation(writer, true)).resolves.toBe('session-new');
    expect(mocks.create).toHaveBeenCalledWith({
      ownerDeviceId: 'ann-device',
      assistantId: 'workspace',
      displayName: 'Writer',
    });
    expect(mocks.open).toHaveBeenCalledWith('session-new');
  });

  it('start a new one on request even when one exists', async () => {
    await expect(startSharedAssistantConversation(writer)).resolves.toBe('session-new');
    expect(mocks.open).toHaveBeenCalledWith('session-new');
  });

  it('replace one whose first message never left this device once it no longer opens', async () => {
    const unsent = { ...withWriter('unsent', 4), state: 'submitting' } as DispatchObserverJob;
    mocks.jobs = { unsent };
    await expect(openSharedAssistantConversationJob(unsent, writer)).resolves.toBe('session-new');
    expect(mocks.removeJob).toHaveBeenCalledWith('unsent');
    expect(mocks.open).toHaveBeenCalledWith('session-new');

    // Still loaded in this run: it is the conversation being written.
    mocks.removeJob.mockReset();
    mocks.sessions.set('session-unsent', {});
    await expect(openSharedAssistantConversationJob(unsent, writer)).resolves.toBe('session-unsent');
    expect(mocks.removeJob).not.toHaveBeenCalled();
  });

  it('do not offer an unsent leftover as the latest when none may start', async () => {
    mocks.jobs = { unsent: { ...withWriter('unsent', 4), state: 'submitting' } };
    await expect(openSharedAssistantConversation(writer, false)).resolves.toBeUndefined();
    expect(mocks.open).not.toHaveBeenCalled();
  });

  it('are deleted, and only they, when the assistant is forgotten', async () => {
    await forgetSharedAssistantConversations(writer);
    expect(mocks.remove.mock.calls.map(([sessionId]) => sessionId)).toEqual(['session-latest', 'session-old']);
  });
});

describe('deleting a conversation with a shared assistant', () => {
  const unsent = () => ({ ...withWriter('unsent', 4), state: 'submitting' }) as DispatchObserverJob;
  const finished = () => ({ ...withWriter('done', 4), state: 'completed' }) as DispatchObserverJob;

  it('keeps one that never reached the owner on this device only', async () => {
    expect(isLocalOnlyConversation(unsent())).toBe(true);
    mocks.sessions.set('session-unsent', { dialogTurns: [] });
    await deleteSharedAssistantConversation(unsent());
    expect(mocks.deleteConversation).not.toHaveBeenCalled();
    expect(mocks.remove).toHaveBeenCalledWith('session-unsent');

    // Once its first message is on the way, the owner has it too.
    mocks.sessions.set('session-unsent', { dialogTurns: [{}] });
    expect(isLocalOnlyConversation(unsent())).toBe(false);
  });

  it('deletes a sent one on the owner first, then its projection here', async () => {
    mocks.sessions.set('session-done', {});
    const order: string[] = [];
    mocks.deleteConversation.mockImplementation(async () => { order.push('owner'); });
    mocks.remove.mockImplementation(async () => { order.push('here'); });
    await deleteSharedAssistantConversation(finished());
    expect(mocks.deleteConversation).toHaveBeenCalledWith('done');
    expect(order).toEqual(['owner', 'here']);
  });

  it('forgets an unloaded one by its job and keeps everything when the owner refuses', async () => {
    await deleteSharedAssistantConversation(finished());
    expect(mocks.dismissJob).toHaveBeenCalledWith('done');

    mocks.dismissJob.mockReset();
    mocks.deleteConversation.mockRejectedValue(new Error('[shared-assistant:active]'));
    await expect(deleteSharedAssistantConversation(finished())).rejects.toThrow('active');
    expect(mocks.dismissJob).not.toHaveBeenCalled();
    expect(mocks.remove).not.toHaveBeenCalled();
  });

  it('asks each owner once whether it deletes, and again after it could not be asked', async () => {
    mocks.support.mockRejectedValueOnce(new Error('offline')).mockResolvedValue({ deleteConversation: true });
    await expect(sharedConversationDeleteSupported('ann-device')).rejects.toThrow('offline');
    await expect(sharedConversationDeleteSupported('ann-device')).resolves.toBe(true);
    await expect(sharedConversationDeleteSupported('ann-device')).resolves.toBe(true);
    expect(mocks.support).toHaveBeenCalledTimes(2);

    mocks.support.mockResolvedValue({ deleteConversation: false });
    await expect(sharedConversationDeleteSupported('bob-device')).resolves.toBe(false);
  });
});
