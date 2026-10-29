import { describe, expect, it } from 'vitest';
import { isAcpExternalAgent, isAgentInOverviewZone, isLocallyManageableSubagent } from './agentVisibility';

describe('agentVisibility', () => {
  it('hides review agents from backend-provided hidden ids', () => {
    expect(
      isAgentInOverviewZone(
        { id: 'ReviewDocs' },
        new Set(['ReviewDocs']),
      ),
    ).toBe(false);
  });

  it('lists ACP agents as external, read-only subagents', () => {
    const acp = { subagentSource: 'external', executionKind: 'acp' };
    expect(isAcpExternalAgent(acp)).toBe(true);
    expect(isLocallyManageableSubagent(acp)).toBe(false);
    expect(isAgentInOverviewZone({ id: 'acp:claude-code', agentKind: 'subagent' })).toBe(true);
  });

  it('treats payloads without an execution kind as native agents', () => {
    expect(isAcpExternalAgent({ subagentSource: 'external' })).toBe(false);
    expect(isAcpExternalAgent({})).toBe(false);
    expect(isAcpExternalAgent(null)).toBe(false);
  });
});
