import { describe, expect, it } from 'vitest';
import type { Session } from '@/flow_chat/types/flow-chat';
import { calledSessionsIn } from './calledSessions';

const session = (sessionId: string, workspaceId: string, lastActiveAt: number, extra: Partial<Session> = {}) => ({
  sessionId, workspaceId, lastActiveAt, config: {}, dialogTurns: [], ...extra,
} as unknown as Session);

describe('calledSessionsIn', () => {
  it('lists only the sessions other assistants started here, newest first', () => {
    const delegatedBy = { assistantName: 'Planner', sessionId: 'caller' };
    const sessions = [
      session('own', 'writer', 30),
      session('older-call', 'writer', 10, { delegatedBy }),
      session('newer-call', 'writer', 20, { delegatedBy }),
      session('elsewhere', 'planner', 40, { delegatedBy }),
    ];
    expect(calledSessionsIn(sessions, 'writer').map(row => row.sessionId)).toEqual(['newer-call', 'older-call']);
    expect(calledSessionsIn(sessions, undefined)).toEqual([]);
  });
});
