import { describe, expect, it } from 'vitest';
import { localAssistantFromAgentType, remoteAgentFromAgentType } from './remoteAgent';

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
