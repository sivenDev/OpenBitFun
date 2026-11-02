// @vitest-environment jsdom
import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import DelegatedTaskList from './DelegatedTaskList';
import { readDelegatedTasks, resetDelegatedTaskReads } from '@/infrastructure/api/service-api/delegatedTaskReads';
import type { WorkspaceInfo } from '@/shared/types';

const view = vi.hoisted(() => ({ activeTabId: 'session' as string | null, activeSessionId: null as string | null }));
const mocks = vi.hoisted(() => ({
  listTasks: vi.fn(),
  warn: vi.fn(),
  openTask: vi.fn(),
  tauri: vi.fn(),
  peer: vi.fn(),
}));

vi.mock('@/infrastructure/api/service-api/DelegationAPI', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/infrastructure/api/service-api/DelegationAPI')>()),
  delegationAPI: { listTasks: mocks.listTasks },
}));
vi.mock('@/app/stores/sceneStore', () => ({
  useSceneStore: (select: (state: { activeTabId: string | null }) => unknown) => select({ activeTabId: view.activeTabId }),
}));
vi.mock('@/shared/notification-system', () => ({ notificationService: { warning: mocks.warn } }));
vi.mock('@/flow_chat/store/FlowChatStore', () => ({
  flowChatStore: { subscribe: () => () => undefined, getState: () => ({ activeSessionId: view.activeSessionId }) },
}));
vi.mock('@/flow_chat/session-drivers/delegated/openDelegatedTask', () => ({ openDelegatedTask: mocks.openTask }));
vi.mock('@/infrastructure/runtime', () => ({ isTauriRuntime: mocks.tauri }));
vi.mock('@/infrastructure/peer-device/peerDeviceContextState', () => ({ usePeerDeviceModeOptional: mocks.peer }));
vi.mock('@/shared/utils/logger', () => ({ createLogger: () => ({ warn: vi.fn() }) }));
vi.mock('@/infrastructure/i18n', () => ({
  useI18n: () => ({
    t: (key: string, values?: Record<string, unknown>) =>
      values ? `${key}:${Object.values(values).join('|')}` : key,
  }),
}));

const ROOT = '/home/me/personal_assistant/workspace';
const workspace = { id: 'ws-1', rootPath: ROOT, name: 'Nova' } as WorkspaceInfo;

function task(jobId: string, overrides: Record<string, unknown> = {}) {
  return {
    jobId, sessionId: `s-${jobId}`, state: 'running', title: `Task ${jobId}`, assistantId: 'workspace',
    agentType: 'assistant:workspace', assistantWorkspace: ROOT, pendingPermissionCount: 0,
    source: { deviceId: '0123', fingerprint: 'AB23CD', displayName: 'Ann', deviceName: 'Laptop' },
    ...overrides,
  };
}

describe('DelegatedTaskList', () => {
  let container: HTMLDivElement;
  let root: Root;
  beforeEach(() => {
    Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
    Object.values(mocks).forEach(mock => mock.mockReset());
    resetDelegatedTaskReads();
    mocks.tauri.mockReturnValue(true);
    mocks.peer.mockReturnValue({ peerMode: { active: false } });
    container = document.createElement('div');
    document.body.append(container);
    root = createRoot(container);
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); });
  const render = async () => { await act(async () => root.render(<DelegatedTaskList workspace={workspace} />)); };
  const rows = () => Array.from(container.querySelectorAll('[data-testid="nav-delegated-task"]'));

  it('lists the tasks this assistant runs for others, with their source', async () => {
    mocks.listTasks.mockResolvedValue([
      task('a', { pendingPermissionCount: 2, source: { deviceId: '4567', fingerprint: 'EF45GH', displayName: 'Bea', deviceName: 'Studio' } }),
      task('b'),
      // The runner may spell the directory differently; the assistant type decides.
      task('c', { assistantWorkspace: `/private${ROOT}` }),
      task('other', { agentType: 'assistant:workspace-work', assistantWorkspace: `${ROOT}-work` }),
    ]);
    await render();
    expect(rows().map(row => row.querySelector('.assistant-session-history__delegated-title')!.textContent))
      .toEqual(['Task a', 'Task b', 'Task c']);
    expect(rows()[0].textContent).toContain('nav.sessions.delegated.from:Bea');
    expect(rows()[0].textContent).toContain('nav.sessions.delegated.waiting:2');
    expect(rows()[0].classList.contains('is-waiting')).toBe(true);
    expect(rows()[1].classList.contains('is-waiting')).toBe(false);
    expect(rows()[1].textContent).toContain('nav.sessions.delegated.from:Ann');

    // A task opens as its own read-only session.
    mocks.openTask.mockResolvedValue(true);
    await act(async () => (rows()[0] as HTMLButtonElement).click());
    expect(mocks.openTask).toHaveBeenCalledWith(expect.objectContaining({ jobId: 'a' }), ROOT, expect.any(String));
    expect(mocks.warn).not.toHaveBeenCalled();

    // Without a session to open, the row says so.
    mocks.openTask.mockResolvedValue(false);
    await act(async () => (rows()[1] as HTMLButtonElement).click());
    expect(mocks.warn).toHaveBeenCalledWith('nav.sessions.delegated.unavailable');
  });

  it('marks the task whose session is shown as selected', async () => {
    view.activeSessionId = 's-b';
    mocks.listTasks.mockResolvedValue([task('a'), task('b')]);
    await render();
    expect(rows().map(row => row.classList.contains('is-active'))).toEqual([false, true]);
    expect(rows()[1].getAttribute('aria-current')).toBe('true');

    // A collapsed history keeps the viewed task visible alongside waiting ones.
    await act(async () => root.render(<DelegatedTaskList workspace={workspace} waitingOnly />));
    expect(rows().map(row => row.querySelector('.assistant-session-history__delegated-title')!.textContent))
      .toEqual(['Task b']);
    view.activeSessionId = null;
  });

  it('lists only tasks waiting for the owner while the history is collapsed', async () => {
    mocks.listTasks.mockResolvedValue([task('a', { pendingPermissionCount: 1 }), task('b')]);
    await act(async () => root.render(<DelegatedTaskList workspace={workspace} waitingOnly />));
    expect(rows().map(row => row.querySelector('.assistant-session-history__delegated-title')!.textContent))
      .toEqual(['Task a']);
  });

  it('follows reads other surfaces start', async () => {
    mocks.listTasks.mockResolvedValue([task('a', { pendingPermissionCount: 1 })]);
    await act(async () => root.render(<DelegatedTaskList workspace={workspace} waitingOnly />));
    expect(rows()).toHaveLength(1);
    mocks.listTasks.mockResolvedValue([task('a')]);
    await act(async () => { await readDelegatedTasks({ fresh: true }); });
    expect(rows()).toHaveLength(0);
  });

  it('shows nothing when there are no tasks or the list cannot be read', async () => {
    mocks.listTasks.mockRejectedValue(new Error('runner missing'));
    await render();
    expect(container.querySelector('[data-testid="nav-delegated-tasks"]')).toBeNull();
  });

  it('lists nothing for a connected peer or outside the desktop app', async () => {
    mocks.peer.mockReturnValue({ peerMode: { active: true } });
    await render();
    mocks.peer.mockReturnValue({ peerMode: { active: false } });
    mocks.tauri.mockReturnValue(false);
    await render();
    expect(mocks.listTasks).not.toHaveBeenCalled();
  });
});
