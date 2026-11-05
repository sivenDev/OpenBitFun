import { describe, expect, it } from 'vitest';
import type { SharedWithMe } from '@/infrastructure/api/service-api/LanShareAPI';
import type { WorkspaceInfo } from '@/shared/types';
import {
  latestEffectiveModel,
  reportedSharedModel,
  requestedAgentTypeForToolCall,
  calledAssistantName,
  calledAssistantOwner,
  calledAssistantSessionName,
  calledTypeInConversation,
  localAssistantFromAgentType,
  sharedConversationOwnerDevice,
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

  it('reads a call in a shared assistant\'s conversation as one to its owner\'s assistant', () => {
    expect(calledTypeInConversation('assistant:workspace-notes', 'laptop'))
      .toBe('shared:laptop/workspace-notes');
    expect(calledTypeInConversation('assistant:workspace-notes', null)).toBe('assistant:workspace-notes');
    expect(calledTypeInConversation('shared:desk/helper', 'laptop')).toBe('shared:desk/helper');
    expect(calledTypeInConversation('Explore', 'laptop')).toBe('Explore');
    // The shared assistant calling one of this device's own assistants back.
    expect(calledTypeInConversation('shared:desk/helper', 'laptop', 'desk')).toBe('assistant:helper');
    expect(calledTypeInConversation('shared:desk/helper', 'laptop', 'phone')).toBe('shared:desk/helper');

    // The owner shares only `helper`; its other assistant is still its own,
    // named by the owner, without a name of its own on this device.
    const list = [shared('laptop', 'helper', 'Writer')];
    const called = calledTypeInConversation('assistant:workspace-notes', 'laptop');
    expect(calledAssistantOwner(called, undefined, list)).toEqual({ kind: 'shared', owner: 'Ann' });
    expect(calledAssistantName(called, [], list)).toBeNull();
  });

  it('finds the owner device of a conversation with a shared assistant only', () => {
    expect(sharedConversationOwnerDevice({
      config: {
        dispatchTarget: {
          kind: 'sharedAssistant', ownerDeviceId: 'laptop', assistantId: 'helper', displayName: 'Writer',
        },
      },
    } as never)).toBe('laptop');
    expect(sharedConversationOwnerDevice({ config: { dispatchTarget: { kind: 'local' } } } as never)).toBeNull();
    expect(sharedConversationOwnerDevice({ config: {} } as never)).toBeNull();
    expect(sharedConversationOwnerDevice(undefined)).toBeNull();
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

  it('says whose assistant a call reaches, and nothing for other agents', () => {
    const list = [shared('laptop', 'helper', 'Writer')];
    expect(calledAssistantOwner('assistant:workspace-x', null, list)).toEqual({ kind: 'mine' });
    expect(calledAssistantOwner('Claw', { delegatedBy: { sessionId: 'caller' } }, list)).toEqual({ kind: 'mine' });
    expect(calledAssistantOwner('shared:laptop/helper', null, list)).toEqual({ kind: 'shared', owner: 'Ann' });
    expect(calledAssistantOwner('shared:desk/gone', null, list)).toEqual({ kind: 'shared', owner: 'desk' });
    expect(calledAssistantOwner('Explore', null, list)).toBeNull();
  });

  it('names a called assistant\'s session by the assistant whose workspace it is in', () => {
    const local = { id: 'ws-writer', rootPath: '/home/me/.openbitfun/personal_assistant/workspace-w', identity: { name: 'Writer' } } as WorkspaceInfo;
    const delegatedBy = { assistant: 'assistant:workspace-r', sessionId: 'caller' };
    expect(calledAssistantSessionName({ delegatedBy, workspaceId: 'ws-writer' }, [local])).toBe('Writer');
    expect(calledAssistantSessionName({ workspaceId: 'ws-writer' }, [local])).toBeNull();
    expect(calledAssistantSessionName({ delegatedBy, workspaceId: 'ws-other' }, [local])).toBeNull();
    expect(calledAssistantSessionName(undefined, [local])).toBeNull();
  });
});

describe('requestedAgentTypeForToolCall', () => {
  it('reads the type a call asked for, which names a called assistant running as Claw', () => {
    const session = { dialogTurns: [{ modelRounds: [{ items: [
      { type: 'text', id: 'text' },
      { type: 'tool', id: 'call-1', toolCall: { id: 'call-1', input: { agent_type: 'assistant:abc' } } },
    ] }] }] } as never;
    expect(requestedAgentTypeForToolCall(session, 'call-1')).toBe('assistant:abc');
    expect(requestedAgentTypeForToolCall(session, 'other')).toBeNull();
    expect(requestedAgentTypeForToolCall(undefined, 'call-1')).toBeNull();
  });
});

describe('latestEffectiveModel', () => {
  it('reads the newest round that recorded the model it ran on', () => {
    const session = { dialogTurns: [
      { modelRounds: [{ effectiveModelName: 'older' }] },
      { modelRounds: [{ effectiveModelName: 'newer' }, { effectiveModelName: ' ' }] },
    ] } as never;
    expect(latestEffectiveModel(session)).toBe('newer');
    expect(latestEffectiveModel({ dialogTurns: [] } as never)).toBeNull();
  });
});

describe('reportedSharedModel', () => {
  const type = 'shared:laptop-1/xiaolan';

  it('names the model the owner reports, never the owner device', () => {
    expect(reportedSharedModel(type, 'gpt-6.1-sol')).toBe('gpt-6.1-sol');
    // Calls recorded by earlier builds named rounds after the device.
    expect(reportedSharedModel(type, 'laptop-1', 'laptop-1')).toBeNull();
    expect(reportedSharedModel(type, 'laptop-1', ' gpt-6.1-terra ')).toBe('gpt-6.1-terra');
    expect(reportedSharedModel(type, undefined, null)).toBeNull();
  });

  it('applies only to shared assistants', () => {
    expect(reportedSharedModel('assistant:xiaolan', 'gpt-6.1-sol')).toBeNull();
  });
});
