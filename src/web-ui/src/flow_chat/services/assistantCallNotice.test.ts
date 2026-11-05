import { describe, expect, it, vi } from 'vitest';
import type { SharedWithMe } from '@/infrastructure/api/service-api/LanShareAPI';
import type { FlowChatState, Session } from '../types/flow-chat';
import { assistantCallNotice } from './assistantCallNotice';

vi.mock('@/infrastructure/services/business/workspaceManager', () => ({
  workspaceManager: { getState: () => ({ assistantWorkspaces: [] }) },
}));

const t = (key: string, values?: Record<string, unknown>) =>
  values ? `${key}:${values.agent}@${values.conversation}` : key;
const shared = [{
  owner: { deviceId: 'device-1', deviceName: 'Mac', displayName: 'agent' },
  assistantId: 'workspace', name: '助理', grantedAt: '', state: 'available', online: true,
}] as unknown as SharedWithMe[];
const session = (sessionId: string, overrides: Partial<Session>) => ({
  sessionId, title: '', config: {}, dialogTurns: [], ...overrides,
} as unknown as Session);

function state(...sessions: Session[]): Pick<FlowChatState, 'sessions'> {
  return { sessions: new Map(sessions.map(value => [value.sessionId, value])) };
}

const options = ({ running = false, visible = false } = {}) => ({
  t, translate: (key: string) => key, sharedWithMe: shared,
  callerIsRunning: () => running, callerIsVisible: () => visible,
});

describe('assistantCallNotice', () => {
  const caller = session('caller', { title: '跨会话沟通机制' });
  const sharedCall = session('call', { parentSessionId: 'caller', subagentType: 'shared:device-1/workspace' });

  it('names the assistant and the conversation that called it when the user is elsewhere', () => {
    expect(assistantCallNotice(state('other', caller, sharedCall), 'call', 'completed', options()))
      .toEqual({ callerSessionId: 'caller', message: 'assistantCallNotice.completed:助理@跨会话沟通机制' });
  });

  it('finds the caller of a local assistant call through who started it', () => {
    const localCall = session('local-call', {
      delegatedBy: { assistant: 'assistant:x', sessionId: 'caller', callChain: [] },
    } as Partial<Session>);
    expect(assistantCallNotice(state(caller, localCall), 'local-call', 'failed', options())?.callerSessionId)
      .toBe('caller');
  });

  it('stays quiet in the caller, while the caller runs, and for other agents', () => {
    expect(assistantCallNotice(state(caller, sharedCall), 'call', 'completed', options({ visible: true }))).toBeNull();
    expect(assistantCallNotice(state(caller, sharedCall), 'call', 'completed', options({ running: true }))).toBeNull();
    const subagent = session('sub', { parentSessionId: 'caller', subagentType: 'Explore' });
    expect(assistantCallNotice(state(caller, subagent), 'sub', 'completed', options())).toBeNull();
  });
});
