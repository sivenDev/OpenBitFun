/**
 * What a conversation with a shared assistant waits on, in a few words for
 * the navigation: the owner's approval, the owner's task limit, or nothing.
 */

import { sharedAssistantProblem } from './types';
import type { DispatchObserverJob } from './dispatchJobStore';

export type SharedAssistantStatus =
  | { kind: 'approval' }
  | { kind: 'queued'; limit?: number }
  | { kind: 'running' }
  | { kind: 'expired' }
  | { kind: 'failed' }
  | { kind: 'idle' };

/**
 * `sent` is false for a new conversation whose first message has not been
 * sent: its job waits in `submitting` without running anything.
 */
export function sharedAssistantStatus(
  job: Pick<DispatchObserverJob, 'state' | 'pendingPermissions' | 'hold' | 'lastError'>,
  sent = true,
): SharedAssistantStatus {
  // The owner answers approvals; until then the task waits on them.
  if ((job.pendingPermissions?.length ?? 0) > 0) return { kind: 'approval' };
  switch (job.state) {
    case 'queued':
      return { kind: 'queued', limit: job.hold?.limit };
    case 'submitting':
      return sent ? { kind: 'running' } : { kind: 'idle' };
    case 'submission_unknown':
    case 'running':
      return { kind: 'running' };
    case 'expired':
      return { kind: 'expired' };
    case 'failed':
      return sharedAssistantProblem(job.lastError) === 'expired' ? { kind: 'expired' } : { kind: 'failed' };
    default:
      return { kind: 'idle' };
  }
}
