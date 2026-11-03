import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { DispatchObserverJob } from './dispatchJobStore';

const mocks = vi.hoisted(() => ({
  jobs: {} as Record<string, unknown>,
  create: vi.fn(),
  remove: vi.fn(),
  open: vi.fn(),
}));
vi.mock('@/flow_chat/services/FlowChatManager', () => ({
  flowChatManager: {
    createSharedAssistantSession: mocks.create,
    deleteChatSession: mocks.remove,
  },
}));
vi.mock('@/flow_chat/services/sessionActivation', () => ({ openMainSession: mocks.open }));
vi.mock('./dispatchJobStore', () => ({
  dispatchJobStore: { getState: () => ({ jobs: mocks.jobs }) },
}));

import {
  forgetSharedAssistantConversations,
  openSharedAssistantConversation,
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
  for (const mock of [mocks.create, mocks.remove, mocks.open]) mock.mockReset();
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

  it('are deleted, and only they, when the assistant is forgotten', async () => {
    await forgetSharedAssistantConversations(writer);
    expect(mocks.remove.mock.calls.map(([sessionId]) => sessionId)).toEqual(['session-latest', 'session-old']);
  });
});
