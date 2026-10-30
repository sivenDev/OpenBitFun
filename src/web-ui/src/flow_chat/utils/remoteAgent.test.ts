import { describe, expect, it } from 'vitest';
import { remoteAgentFromAgentType } from './remoteAgent';

describe('remoteAgentFromAgentType', () => {
  it('splits the device and open agent on the last separator', () => {
    expect(remoteAgentFromAgentType('remote:laptop/helper')).toEqual({
      deviceId: 'laptop',
      openAgentId: 'helper',
    });
    expect(remoteAgentFromAgentType('remote:org/device/helper')?.deviceId).toBe('org/device');
  });

  it('rejects other and malformed agent types', () => {
    for (const value of ['acp:claude-code', 'Standard', 'remote:', 'remote:laptop', 'remote:/helper', 'remote:laptop/', null]) {
      expect(remoteAgentFromAgentType(value)).toBeNull();
    }
  });
});
