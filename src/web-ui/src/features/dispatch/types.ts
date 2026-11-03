export type DispatchTargetRequest =
  | { kind: 'local' }
  | { kind: 'ssh'; connectionId: string; workspacePath: string }
  | { kind: 'device'; deviceId: string; workspacePath: string }
  /**
   * An assistant another device shares on the local network. It runs in that
   * device's workspace for this one: no Git baseline, nothing to sync back.
   */
  | { kind: 'sharedAssistant'; ownerDeviceId: string; assistantId: string };

export type DispatchTarget =
  | { kind: 'local' }
  | {
      kind: 'ssh';
      connectionId: string;
      workspacePath: string;
      displayName: string;
    }
  | {
      kind: 'device';
      deviceId: string;
      workspacePath: string;
      displayName: string;
    }
  | {
      kind: 'sharedAssistant';
      ownerDeviceId: string;
      assistantId: string;
      /** The assistant's name as its owner shares it. */
      displayName: string;
    };

/** Targets reached over a Git baseline, which a target picker offers. */
export type GitDispatchTargetRequest = Exclude<
  DispatchTargetRequest,
  { kind: 'local' } | { kind: 'sharedAssistant' }
>;
export type GitDispatchTarget = Exclude<
  DispatchTarget,
  { kind: 'local' } | { kind: 'sharedAssistant' }
>;

export type DispatchApprovalPolicy = 'auto' | 'reject-and-report' | 'remote';

/**
 * How a dispatch reaches its target: a Git worktree of the controller's own
 * repository, checked out on the target at the same commit.
 *
 * Recorded on the session so the sync button knows where to fetch the target's
 * branch back into. It is resolved by the backend at submit time, not chosen in
 * the UI — the UI only decides `includeUncommitted`.
 */
export interface DispatchWorkspaceDelivery {
  sourceWorkspacePath: string;
  baselineWorktreeId: string;
  baseCommit: string;
  branch: string;
  remoteUrl?: string;
  includeUncommitted: boolean;
}
export type DispatchReachability = 'unknown' | 'reachable' | 'unreachable';
export type DispatchJobState =
  | 'submitting'
  | 'submission_unknown'
  | 'queued'
  | 'running'
  | 'succeeded'
  | 'failed'
  | 'cancelled'
  /** A shared assistant's owner no longer keeps the conversation. */
  | 'expired';

export interface DispatchTargetOption {
  kind: 'local' | 'ssh' | 'device';
  connectionId?: string;
  deviceId?: string;
  displayName: string;
  description?: string;
  online?: boolean;
  /** Relay-confirmed client incompatibility; `true` blocks device dispatch. */
  incompatible?: boolean;
}

export interface DispatchCliRelease {
  version: string;
  target: string;
  url: string;
  sha256: string;
}

export interface DispatchWorkspaceProbe {
  path: string;
  exists: boolean;
  isDirectory: boolean;
  isGitRepository: boolean;
  /**
   * Git found the repository but refuses to read it until the target host's
   * user trusts the path. Only then is `branch`/`dirty` absence a permission
   * problem rather than a plain unborn branch or clean tree.
   */
  trustRequired?: boolean;
  branch?: string;
  dirty?: boolean;
  ahead?: number;
  behind?: number;
}

export interface DispatchProtocolProbe {
  productId: string;
  dataNamespace: string;
  protocolVersion: number;
  cliVersion: string;
  os: string;
  arch: string;
  capabilities: string[];
  modelConfigured: boolean;
  availableModels: string[];
  /** Canonical capability projection produced by the target runtime. */
  modelCatalog: import('@/infrastructure/api/service-api/AIApi').AIModelCatalog;
  defaultModel?: string;
  modelDiagnostic?: string;
  workspace?: DispatchWorkspaceProbe;
}

export interface DispatchSshProbe {
  cliInstalled: boolean;
  cliPath?: string;
  os: string;
  arch: string;
  installSupported: boolean;
  installError?: string;
  protocolError?: string;
  release?: DispatchCliRelease;
  protocol?: DispatchProtocolProbe;
  /** Set when no published binary can run on this target, with the reason. */
  prebuiltIncompatible?: string;
}

export interface DispatchSyncedChange {
  /** Git name-status letter: `A`, `M`, `D`, and so on. */
  status: string;
  path: string;
}

/**
 * Outcome of one sync-back.
 *
 * `changed: false` means the target's worktree still matches `baseCommit` — no
 * bundle was built and nothing was fetched.
 */
export interface DispatchSyncResult {
  changed: boolean;
  branch: string;
  baseCommit: string;
  headCommit: string;
  commitCount: number;
  changes: DispatchSyncedChange[];
  /** True when the change list was capped; the fetched history is complete. */
  truncatedChanges: boolean;
  /** Controller worktree the branch was fast-forwarded into. */
  baselineWorktreePath?: string;
  syncedHeadCommit?: string;
}

export interface DispatchInstallStart {
  scriptPath: string;
  version: string;
  target: string;
  url: string;
  sha256: string;
}

export interface DispatchInstallPoll {
  cursor: number;
  output: string;
  status: 'running' | 'succeeded' | 'failed';
}

export interface DispatchProvisionTargetResult {
  accountStatus: 'synced' | 'skipped_not_logged_in';
  daemonInstalled: boolean;
}

export type DispatchEvent =
  | {
      type: 'audit';
      timestamp: string;
      action: string;
      details: Record<string, unknown>;
    }
  | {
      type: 'jobState';
      timestamp: string;
      state: Exclude<DispatchJobState, 'submitting'>;
      message?: string;
    }
  | {
      type: 'agentEvent';
      timestamp: string;
      event: DispatchAgentEventEnvelope | Record<string, unknown>;
      frontendEventName?: string;
      frontendPayload?: Record<string, unknown>;
    }
  | {
      type: 'permissionRejected';
      timestamp: string;
      request: Record<string, unknown>;
      reason: string;
    };

export interface DispatchAgentEventEnvelope {
  id?: string;
  event?: Record<string, unknown>;
  priority?: string | number;
  timestamp?: unknown;
  frontendEventName?: string;
  frontendPayload?: Record<string, unknown>;
}

export interface DispatchSubmitResponse {
  accepted: boolean;
  jobId: string;
  sessionId: string;
  state: Exclude<DispatchJobState, 'submitting'>;
}

export interface DispatchStatusResponse {
  state: Exclude<DispatchJobState, 'submitting'>;
  cursor: number;
  events: DispatchEvent[];
  pendingPermissions: Array<Record<string, unknown>>;
  cursorReset: boolean;
  historyTruncated: boolean;
  eventLogComplete: boolean;
  omittedEventCount: number;
  lastError?: string;
  /** Why a queued job waits, when its target says. Absent on older targets. */
  hold?: DispatchHold;
}

/** Why a queued job waits on its target. */
export interface DispatchHold {
  /** `shareLimit`: the shared assistant runs as many tasks as its owner allows. */
  reason: string;
  /** How many tasks the owner allows at once, for a limit. */
  limit?: number;
  /** When it began to wait, in milliseconds since the epoch. */
  sinceMs: number;
}

export interface DispatchContinueResponse {
  accepted: boolean;
  jobId: string;
  sessionId: string;
  turnId: string;
  state: Exclude<DispatchJobState, 'submitting'>;
}

export interface DispatchCancelResponse {
  cancelled: boolean;
}

export interface DispatchJobListEntry {
  jobId: string;
  sessionId: string;
  state: Exclude<DispatchJobState, 'submitting'>;
  startedAt?: string;
  workspacePath: string;
  title: string;
  agentType?: string;
  approvalPolicy?: DispatchApprovalPolicy;
  model?: string;
  reasoningPreset?: string;
}

export interface OutboundDispatchRecord {
  jobId: string;
  target: DispatchTarget;
  sessionId: string;
  /** Controller workspace that owns the observer session. */
  sourceWorkspacePath?: string;
  sourceWorkspaceId?: string;
  /** Managed worktree this job branched from; absent on pre-Git records. */
  baselineWorktreeId?: string;
  baselineWorktreePath?: string;
  /** Stable main checkout that owns the managed-worktree registry. */
  baselineProjectWorkspacePath?: string;
  baseCommit?: string;
  branch?: string;
  remoteUrl?: string;
  /** Branch tip the last successful sync fetched. */
  syncedHeadCommit?: string;
  workspacePath: string;
  promptPreview: string;
  title?: string;
  agentType?: string;
  approvalPolicy?: DispatchApprovalPolicy;
  model?: string;
  reasoningPreset?: string;
  lastCursor: number;
  lastState: DispatchJobState;
  createdAt: string;
  updatedAt: string;
}

/**
 * Bumped whenever the projection this cache stores changes shape or meaning.
 *
 * A mismatch discards the cache and replays the job from byte zero, so this is
 * the only thing standing between a projection change and a transcript rendered
 * by rules that no longer exist.
 */
// v5 persists title projection metadata; old caches replay title events once.
export const DISPATCH_TRANSCRIPT_SCHEMA_VERSION = 5;

/**
 * The controller's UI cache for one observer projection.
 *
 * Cursor, turns, and completeness facts are one document on purpose: the cursor
 * is only meaningful next to the turns it produced, and rendering a truncated
 * transcript as a complete one is exactly what dispatch invariant 14 forbids.
 */
export interface DispatchTranscriptCache {
  schemaVersion: number;
  title?: string;
  titleSource?: 'generated' | 'manual';
  jobId: string;
  sessionId: string;
  cursor: number;
  dialogTurns: unknown[];
  appliedEventIds: string[];
  eventLogComplete: boolean;
  historyTruncated: boolean;
  omittedEventCount: number;
}

export interface DispatchSelection {
  request: GitDispatchTargetRequest;
  target: GitDispatchTarget;
  /** Fold the baseline worktree's uncommitted changes into the base commit. */
  includeUncommitted: boolean;
  /** Git revision resolved when creating the controller baseline worktree. */
  baseRef: string;
  /**
   * No approval policy and no model: both are ordinary composer controls that
   * protocol v4 carries per turn, so picking a target decides only what the
   * target cannot change later — where it runs and which code it gets.
   */
  model?: string;
  modelCatalog?: import('@/infrastructure/api/service-api/AIApi').AIModelCatalog;
  availableModels?: string[];
  defaultModel?: string;
}

export function isNonLocalDispatchTarget(
  target: DispatchTargetRequest | DispatchTarget | undefined,
): target is Exclude<DispatchTargetRequest | DispatchTarget, { kind: 'local' }> {
  return !!target && target.kind !== 'local';
}

export function isDispatchJobTerminal(state: DispatchJobState | undefined): boolean {
  return state === 'succeeded' || state === 'failed' || state === 'cancelled' || state === 'expired';
}

/**
 * Job ids of conversations with shared assistants start with this; their
 * owners list and answer them as tasks from other devices.
 */
export const SHARED_ASSISTANT_JOB_ID_PREFIX = 'delegation-';

/** Every reason a conversation with a shared assistant cannot go on. */
export const SHARED_ASSISTANT_PROBLEMS = [
  'notShared',
  'revoked',
  'offline',
  'needsUpdate',
  'expired',
  'unavailable',
] as const;

/** Why a conversation with a shared assistant cannot go on. */
export type SharedAssistantProblem = typeof SHARED_ASSISTANT_PROBLEMS[number];

const SHARED_ASSISTANT_PROBLEM = new RegExp(
  `\\[shared-assistant:(${SHARED_ASSISTANT_PROBLEMS.join('|')})\\]`,
);

/**
 * The reason a shared-assistant call failed, from the `[shared-assistant:<code>]`
 * mark the backend puts on such errors.
 */
export function sharedAssistantProblem(error: unknown): SharedAssistantProblem | undefined {
  if (error instanceof Error && 'sharedAssistantProblem' in error) {
    return (error as Error & { sharedAssistantProblem: SharedAssistantProblem }).sharedAssistantProblem;
  }
  const message = error instanceof Error ? error.message : String(error ?? '');
  return SHARED_ASSISTANT_PROBLEM.exec(message)?.[1] as SharedAssistantProblem | undefined;
}
