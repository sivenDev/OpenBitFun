import { describe, expect, it } from 'vitest';
import type { SharedWithMe } from '@/infrastructure/api/service-api/LanShareAPI';
import type { WorkspaceInfo } from '@/shared/types';
import {
  calledAssistantName,
  localAssistantFromAgentType,
  localAssistantName,
  remoteAgentFromAgentType,
  sharedAssistantNames,
} from './remoteAgent';

describe('remoteAgentFromAgentType', () => {
  it('splits the owner device and assistant on the last separator', () => {
    expect(remoteAgentFromAgentType('shared:laptop/helper')).toEqual({
      deviceId: 'laptop',
      assistantId: 'helper',
    });
    expect(remoteAgentFromAgentType('shared:org/device/helper')?.deviceId).toBe('org/device');
  });

  it('rejects other and malformed agent types', () => {
    for (const value of ['acp:claude-code', 'Standard', 'assistant:workspace', 'remote:laptop/helper', 'shared:', 'shared:laptop', 'shared:/helper', 'shared:laptop/', null]) {
      expect(remoteAgentFromAgentType(value)).toBeNull();
    }
  });
});

describe('localAssistantFromAgentType', () => {
  it('reads the assistant id of this device\'s assistants only', () => {
    expect(localAssistantFromAgentType('assistant:workspace-writer')).toBe('workspace-writer');
    for (const value of ['assistant:', 'shared:laptop/helper', 'Standard', undefined]) {
      expect(localAssistantFromAgentType(value)).toBeNull();
    }
  });
});

describe('assistant names', () => {
  const shared = (deviceId: string, assistantId: string, name: string): SharedWithMe => ({
    owner: { publicKey: 'ab', deviceId, fingerprint: 'AB23CD', displayName: 'Ann', deviceName: "Ann's Mac" },
    assistantId,
    name,
    grantedAt: '2026-11-01T00:00:00Z',
    state: 'available',
    online: true,
  });

  it('finds a shared assistant by owner device and folder id', () => {
    const list = [shared('desk', 'helper', 'Other'), shared('laptop', 'helper', 'Writer')];
    expect(sharedAssistantNames({ deviceId: 'laptop', assistantId: 'helper' }, list))
      .toEqual({ name: 'Writer', owner: 'Ann' });
    expect(sharedAssistantNames({ deviceId: 'laptop', assistantId: 'gone' }, list)).toBeNull();
    expect(sharedAssistantNames({ deviceId: 'laptop', assistantId: 'helper' }, [])).toBeNull();
  });

  it('finds a local assistant by its folder, never a remote workspace', () => {
    const local = { rootPath: '/home/me/.openbitfun/personal_assistant/workspace-x', identity: { name: 'Planner' } } as WorkspaceInfo;
    const remote = { rootPath: '/srv/workspace-y', connectionId: 'ssh-1', identity: { name: 'Remote' } } as WorkspaceInfo;
    expect(localAssistantName('workspace-x', [remote, local])).toBe('Planner');
    expect(localAssistantName('workspace-y', [remote, local])).toBeNull();
    expect(localAssistantName('workspace-x', null)).toBeNull();
  });

  it('names the assistant an agent type calls, and nothing for other agents', () => {
    const local = { rootPath: '/home/me/.openbitfun/personal_assistant/workspace-x', identity: { name: 'Planner' } } as WorkspaceInfo;
    const list = [shared('laptop', 'helper', 'Writer')];
    expect(calledAssistantName('assistant:workspace-x', [local], list)).toBe('Planner');
    expect(calledAssistantName('shared:laptop/helper', [local], list)).toBe('Writer');
    expect(calledAssistantName('Explore', [local], list)).toBeNull();
    expect(calledAssistantName('assistant:workspace-gone', [local], list)).toBeNull();
  });
});
