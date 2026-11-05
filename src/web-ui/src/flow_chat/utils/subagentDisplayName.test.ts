import { describe, expect, it, vi } from 'vitest';
import type { SharedWithMe } from '@/infrastructure/api/service-api/LanShareAPI';
import { resolveSubagentNameKey } from '../subagent-identity';
import { subagentDisplayName } from './subagentDisplayName';

vi.mock('@/infrastructure/services/business/workspaceManager', () => ({
  workspaceManager: { getState: () => ({ assistantWorkspaces: [] }) },
}));

const t = (key: string) => key;
const shared = [{
  owner: { deviceId: 'device-1', deviceName: 'Mac', displayName: 'agent' },
  assistantId: 'workspace', name: '助理', grantedAt: '', state: 'available', online: true,
}] as unknown as SharedWithMe[];

describe('subagentDisplayName', () => {
  it('names a call to a shared assistant after the assistant, not a nickname', () => {
    const session = { subagentType: 'shared:device-1/workspace', config: {} };
    expect(subagentDisplayName(t, 'child', session, shared)).toBe('助理');
    expect(subagentDisplayName(t, 'child', session, [])).toBe('flowChatHeader.agentTreeSharedAssistant');
  });

  it('keeps the generated nickname for other agents', () => {
    expect(subagentDisplayName(t, 'child', { subagentType: 'Explore', config: {} }, shared))
      .toBe(resolveSubagentNameKey('child'));
  });
});
