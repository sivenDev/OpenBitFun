import { describe, expect, it } from 'vitest';
import { sharedAssistantStatus } from './sharedAssistantStatus';

const job = (state: Parameters<typeof sharedAssistantStatus>[0]['state'], extra: object = {}) => ({
  state,
  pendingPermissions: [],
  ...extra,
});

describe('what a conversation with a shared assistant waits on', () => {
  it('is the owner while an approval is pending, whatever the job state', () => {
    expect(sharedAssistantStatus(job('running', { pendingPermissions: [{ requestId: 'p' }] }))).toEqual({ kind: 'approval' });
  });

  it('is the owner\'s task limit while queued, when the owner says so', () => {
    expect(sharedAssistantStatus(job('queued', { hold: { reason: 'shareLimit', limit: 2, sinceMs: 1 } }))).toEqual({ kind: 'queued', limit: 2 });
    expect(sharedAssistantStatus(job('queued'))).toEqual({ kind: 'queued', limit: undefined });
  });

  it('reads running, expired, failed and settled jobs', () => {
    expect(sharedAssistantStatus(job('submitting'))).toEqual({ kind: 'running' });
    expect(sharedAssistantStatus(job('running'))).toEqual({ kind: 'running' });
    expect(sharedAssistantStatus(job('expired'))).toEqual({ kind: 'expired' });
    expect(sharedAssistantStatus(job('failed', { lastError: '[shared-assistant:expired] gone' }))).toEqual({ kind: 'expired' });
    expect(sharedAssistantStatus(job('failed', { lastError: 'boom' }))).toEqual({ kind: 'failed' });
    expect(sharedAssistantStatus(job('succeeded'))).toEqual({ kind: 'idle' });
  });

  it('reads nothing for a new conversation before its first message is sent', () => {
    expect(sharedAssistantStatus(job('submitting'), false)).toEqual({ kind: 'idle' });
    expect(sharedAssistantStatus(job('running'), false)).toEqual({ kind: 'running' });
  });
});
