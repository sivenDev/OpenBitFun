import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { DispatchObserverJob } from './dispatchJobStore';

const mocks = vi.hoisted(() => ({
  jobs: {} as Record<string, unknown>,
  create: vi.fn(),
  remove: vi.fn(),
  open: vi.fn(),
  removeJob: vi.fn(),
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
  dispatchJobStore: { getState: () => ({ jobs: mocks.jobs, removeJob: mocks.removeJob }) },
}));
vi.mock('@/flow_chat/store/FlowChatStore', () => ({
  flowChatStore: { getState: () => ({ sessions: mocks.sessions }) },
}));

import {
  forgetSharedAssistantConversations,
  openSharedAssistantConversation,
  openSharedAssistantConversationJob,
  sharedAssistantConversations,
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
  for (const mock of [mocks.create, mocks.remove, mocks.open, mocks.removeJob]) mock.mockReset();
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
