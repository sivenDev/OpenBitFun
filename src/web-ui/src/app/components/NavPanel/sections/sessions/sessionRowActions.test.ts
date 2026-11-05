import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Session } from '@/flow_chat/types/flow-chat';

const mocks = vi.hoisted(() => ({
  deleteChatSession: vi.fn(),
  clearDefault: vi.fn(),
  driver: vi.fn(),
  error: vi.fn(),
  warning: vi.fn(),
}));
vi.mock('@/flow_chat/services/FlowChatManager', () => ({ flowChatManager: { deleteChatSession: mocks.deleteChatSession } }));
vi.mock('@/flow_chat/session-drivers/resolve', () => ({ resolveSessionDriverId: mocks.driver }));
vi.mock('@/app/components/NavPanel/sections/workspaces/assistantSessionPreferences', () => ({
  clearAssistantDefaultSession: mocks.clearDefault,
}));
vi.mock('@/infrastructure/services/business/workspaceManager', () => ({ workspaceManager: {} }));
vi.mock('@/shared/notification-system', () => ({ notificationService: { error: mocks.error, warning: mocks.warning } }));

import { canDeleteEmptySession, deleteEmptySessionRow } from './sessionRowActions';

const t = (key: string) => key;

function session(overrides: Partial<Session> = {}): Session {
  return {
    sessionId: 's-1',
    workspaceId: 'assistant',
    sessionKind: 'normal',
    dialogTurns: [],
    historyState: 'new',
    config: {},
    ...overrides,
  } as unknown as Session;
}

const row = { isAssistant: true, isChild: false, isRunning: false };

beforeEach(() => {
  vi.resetAllMocks();
  mocks.driver.mockReturnValue('local');
});

describe('deleting an empty assistant session from its menu', () => {
  it('is offered for an assistant\'s own top-level session that sent nothing', () => {
    expect(canDeleteEmptySession(session(), row)).toBe(true);
    expect(canDeleteEmptySession(session({ draft: { phase: 'editing' } } as Partial<Session>), row)).toBe(true);
  });

  it('is not offered for sessions with content, or that are not the assistant\'s own', () => {
    expect(canDeleteEmptySession(session({ historyState: 'ready', totalTurnCount: 2 } as Partial<Session>), row)).toBe(false);
    expect(canDeleteEmptySession(session(), { ...row, isAssistant: false })).toBe(false);
    expect(canDeleteEmptySession(session(), { ...row, isChild: true })).toBe(false);
    expect(canDeleteEmptySession(session(), { ...row, isRunning: true })).toBe(false);
    expect(canDeleteEmptySession(session({ parentSessionId: 'parent' }), row)).toBe(false);
    expect(canDeleteEmptySession(session({ sessionKind: 'subagent' } as Partial<Session>), row)).toBe(false);
    expect(canDeleteEmptySession(session({ persistedStatus: 'archived' } as Partial<Session>), row)).toBe(false);
    expect(canDeleteEmptySession(session({ importedSource: 'claude-code' } as Partial<Session>), row)).toBe(false);
    expect(canDeleteEmptySession(session({ isTransient: true } as Partial<Session>), row)).toBe(false);
    mocks.driver.mockReturnValue('dispatch');
    expect(canDeleteEmptySession(session(), row)).toBe(false);
    mocks.driver.mockReturnValue('delegated');
    expect(canDeleteEmptySession(session(), row)).toBe(false);
  });

  it('deletes the session, then clears it as the assistant default', async () => {
    mocks.deleteChatSession.mockResolvedValue(undefined);
    mocks.clearDefault.mockResolvedValue(undefined);
    await deleteEmptySessionRow('s-1', 'assistant', t);
    expect(mocks.deleteChatSession).toHaveBeenCalledWith('s-1');
    expect(mocks.clearDefault).toHaveBeenCalledWith('assistant', 's-1');
    expect(mocks.deleteChatSession.mock.invocationCallOrder[0]).toBeLessThan(mocks.clearDefault.mock.invocationCallOrder[0]);
    expect(mocks.error).not.toHaveBeenCalled();
  });

  it('keeps the default when the deletion fails, and says so', async () => {
    mocks.deleteChatSession.mockRejectedValue(new Error('Host offline'));
    await deleteEmptySessionRow('s-1', 'assistant', t);
    expect(mocks.clearDefault).not.toHaveBeenCalled();
    expect(mocks.error).toHaveBeenCalledWith('nav.sessions.deleteFailed', expect.anything());
  });

  it('keeps the deletion when only clearing the default fails', async () => {
    mocks.deleteChatSession.mockResolvedValue(undefined);
    mocks.clearDefault.mockRejectedValue(new Error('Host offline'));
    await deleteEmptySessionRow('s-1', 'assistant', t);
    expect(mocks.error).not.toHaveBeenCalled();
    expect(mocks.warning).toHaveBeenCalledWith('nav.sessions.defaultSaveFailed', expect.anything());
  });
});
