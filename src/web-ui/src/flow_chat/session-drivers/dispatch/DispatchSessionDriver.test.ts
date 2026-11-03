import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  submit: vi.fn(),
  registerJob: vi.fn(),
  updateProgress: vi.fn(),
  refresh: vi.fn(),
}));
vi.mock('@/features/dispatch/dispatchApi', () => ({ dispatchApi: { submit: mocks.submit } }));
vi.mock('@/features/dispatch/dispatchJobStore', () => ({
  dispatchJobStore: {
    getState: () => ({ registerJob: mocks.registerJob, updateProgress: mocks.updateProgress, jobs: {} }),
  },
}));
vi.mock('@/features/dispatch/DispatchJobObserver', () => ({ requestDispatchJobRefresh: mocks.refresh }));
vi.mock('@/infrastructure/i18n', () => ({
  i18nService: { t: (key: string, params?: Record<string, unknown>) => (params ? `${key} ${Object.values(params).join(' ')}` : key) },
}));
vi.mock('@/shared/utils/logger', () => ({
  createLogger: () => ({ error: vi.fn(), warn: vi.fn(), info: vi.fn(), debug: vi.fn() }),
}));
vi.mock('../shared', () => ({ addSubmittedDialogTurn: vi.fn(), applyGeneratingTitlePlaceholder: vi.fn() }));
vi.mock('../../store/runtimeStatusStore', () => ({ showRuntimeStatus: vi.fn(), clearRuntimeStatusState: vi.fn() }));

import { dispatchSessionDriver } from './DispatchSessionDriver';
import type { SessionConfig } from '../../services/flow-chat-manager/types';
import type { Session } from '../../types/flow-chat';

const surfaceScope = { assertCurrent: vi.fn() };
const createSession = vi.fn();
const context = {
  flowChatStore: {
    createSession,
    applyDispatchSnapshot: vi.fn(),
    updateSessionLastSubmittedMode: vi.fn(),
  },
} as never;

const sharedConfig: SessionConfig = {
  dispatchTargetRequest: { kind: 'sharedAssistant', ownerDeviceId: 'ann-device', assistantId: 'writer' },
  dispatchTarget: { kind: 'sharedAssistant', ownerDeviceId: 'ann-device', assistantId: 'writer', displayName: 'Writer' },
  dispatchApprovalPolicy: 'remote',
  // Whatever a caller passes, a shared assistant takes no Git baseline.
  dispatchBaseRef: 'main',
  dispatchIncludeUncommitted: true,
};

beforeEach(() => {
  for (const mock of [...Object.values(mocks), createSession]) mock.mockReset();
});

describe('a conversation with a shared assistant', () => {
  it('is created without a workspace or a Git baseline, under a shared-assistant job id', async () => {
    const sessionId = await dispatchSessionDriver.createSession(context, {
      surfaceScope: surfaceScope as never,
      config: sharedConfig,
      agentType: 'assistant:writer',
      sessionName: 'New conversation',
      titleDescriptor: { text: 'New conversation' } as never,
      workspacePath: '/Users/me/project',
      projectWorkspacePath: '/Users/me/project',
      workspaceId: 'workspace-1',
    });

    const [createdId, config, , , , , workspacePath] = createSession.mock.calls[0];
    expect(createdId).toBe(sessionId);
    expect(workspacePath).toBeUndefined();
    expect(config).toMatchObject({
      workspaceId: undefined,
      workspacePath: undefined,
      projectWorkspacePath: undefined,
      dispatchBaseRef: undefined,
      dispatchIncludeUncommitted: false,
    });
    expect(config.dispatchJobId).toMatch(/^delegation-/);
    expect(mocks.registerJob).toHaveBeenCalledWith(expect.objectContaining({
      jobId: config.dispatchJobId,
      sourceWorkspacePath: undefined,
      sourceWorkspaceId: undefined,
    }));
  });

  it('is submitted without a Git baseline, and a marked failure reads as its reason', async () => {
    const session = {
      sessionId: 'session-1',
      config: { ...sharedConfig, dispatchJobId: 'delegation-1', dispatchJobState: 'submitting' },
      dialogTurns: [],
    } as unknown as Session;
    const input = {
      surfaceScope,
      sessionId: 'session-1',
      message: 'Hello',
      currentAgentType: 'assistant:writer',
      acpClientId: null,
      isFirstMessage: false,
      readySession: session,
    } as never;
    const tracker = {} as never;

    mocks.submit.mockResolvedValue({ accepted: true, jobId: 'delegation-1', sessionId: 'session-1', state: 'queued' });
    await expect(dispatchSessionDriver.startTurn(context, input, tracker)).resolves.toBe('completed');
    const request = mocks.submit.mock.calls[0][0];
    expect(request).not.toHaveProperty('baseRef');
    expect(request.includeUncommitted).toBe(false);
    expect(request).not.toHaveProperty('sourceWorkspacePath');

    const tooLarge = { ...input, options: { imageContexts: [{ id: 'i', mime_type: 'image/png', data_url: 'x'.repeat(200 * 1024) }] } };
    await expect(dispatchSessionDriver.startTurn(context, tooLarge, tracker)).rejects.toThrow(
      'flow-chat:chatInput.dispatch.errors.deviceAttachmentTooLarge',
    );

    mocks.submit.mockRejectedValue(new Error('[shared-assistant:offline] connection refused'));
    await expect(dispatchSessionDriver.startTurn(context, input, tracker)).rejects.toThrow(
      'flow-chat:chatInput.dispatch.sharedAssistant.problems.offline Writer',
    );
  });
});
