import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ answerTask: vi.fn() }));

vi.mock('@/infrastructure/api/service-api/DelegationAPI', () => ({ delegationAPI: { answerTask: mocks.answerTask } }));
vi.mock('@/infrastructure/i18n', () => ({ i18nService: { t: (key: string) => key } }));
vi.mock('../../services/flow-chat-manager/PersistenceModule', () => ({ cleanupSaveState: vi.fn() }));
vi.mock('../../services/flow-chat-manager/TextChunkModule', () => ({ cleanupSessionBuffers: vi.fn() }));
vi.mock('./delegatedTaskFollower', () => ({ forgetDelegatedTask: vi.fn() }));

import { delegatedSessionDriver } from './DelegatedSessionDriver';
import { delegatedSessionStore } from './delegatedSessionStore';
import { deriveSessionConversationCapability } from '../conversationCapability';
import { isObserverSessionDriver, registerDriverSessionLookup, resolveSessionDriverId } from '../resolve';

const request = (requestId: string) => ({
  requestId, sessionId: 'session-1', action: 'bash', resources: ['ls'], saveResources: ['ls'],
});

describe('delegated session driver', () => {
  beforeEach(() => {
    mocks.answerTask.mockResolvedValue(undefined);
    delegatedSessionStore.getState().register({
      jobId: 'delegation-1', sessionId: 'session-1', cursor: 0, state: 'running',
      pendingPermissions: [request('r1'), request('r2')],
    });
  });

  afterEach(() => {
    delegatedSessionStore.getState().remove('session-1');
    registerDriverSessionLookup(() => undefined);
  });

  it('owns the task session and its subagents, read-only', () => {
    registerDriverSessionLookup(id => (id === 'child-1' ? { parentSessionId: 'session-1' } : undefined));
    expect(resolveSessionDriverId('session-1', undefined)).toBe('delegated');
    expect(resolveSessionDriverId('child-1', { parentSessionId: 'session-1' })).toBe('delegated');
    expect(resolveSessionDriverId('other', undefined)).toBe('local');
    expect(isObserverSessionDriver('delegated')).toBe(true);
    expect(deriveSessionConversationCapability({ config: {} } as never, 'delegated'))
      .toEqual({ access: 'read-only', canSubmit: false, reason: 'unsupported_route' });
    expect(delegatedSessionDriver.planSubmission({} as never, 'session-1', { message: 'hi' }).kind).toBe('reject');
  });

  it('approves one action at a time and hides it once answered', async () => {
    const source = delegatedSessionDriver.permissionRequestSource('session-1');
    if (source === 'live') throw new Error('expected a store source');
    const first = source.getSnapshot();
    expect(first.map(item => (item as { requestId: string }).requestId)).toEqual(['r1', 'r2']);
    // No saved grants on another account's behalf.
    expect(first.every(item => !('saveResources' in item))).toBe(true);
    expect(source.getSnapshot()).toBe(first);

    await delegatedSessionDriver.respondPermission('session-1', 'r1', 'always');
    expect(mocks.answerTask).toHaveBeenCalledWith('delegation-1', 'r1', 'once');
    expect(source.getSnapshot().map(item => (item as { requestId: string }).requestId)).toEqual(['r2']);

    // The runner still lists r1 until it consumes the answer.
    delegatedSessionStore.getState().update('session-1', { pendingPermissions: [request('r1'), request('r2')] });
    expect(source.getSnapshot().map(item => (item as { requestId: string }).requestId)).toEqual(['r2']);

    await delegatedSessionDriver.respondPermission('session-1', 'r2', 'reject');
    expect(mocks.answerTask).toHaveBeenLastCalledWith('delegation-1', 'r2', 'reject');
  });
});
