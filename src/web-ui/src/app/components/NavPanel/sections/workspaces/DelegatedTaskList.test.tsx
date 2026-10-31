// @vitest-environment jsdom
import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import DelegatedTaskList from './DelegatedTaskList';
import { resetDelegatedTaskReads } from './delegatedTaskReads';
import type { WorkspaceInfo } from '@/shared/types';

const mocks = vi.hoisted(() => ({
  listTasks: vi.fn(),
  openScene: vi.fn(),
  tauri: vi.fn(),
  peer: vi.fn(),
}));

vi.mock('@/infrastructure/api/service-api/DelegationAPI', () => ({ delegationAPI: { listTasks: mocks.listTasks } }));
vi.mock('@/app/stores/sceneStore', () => ({ useSceneStore: { getState: () => ({ openScene: mocks.openScene }) } }));
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
    jobId, sessionId: `s-${jobId}`, state: 'running', title: `Task ${jobId}`, openAgentId: 'workspace',
    agentType: 'assistant:workspace', assistantWorkspace: ROOT, pendingPermissionCount: 0,
    source: { kind: 'device', deviceId: 'laptop', deviceName: 'Laptop' },
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
      task('a', { pendingPermissionCount: 2, source: { kind: 'connection', connectionId: 'c1', accountName: 'Bea' } }),
      task('b'),
      task('other', { assistantWorkspace: `${ROOT}-work` }),
    ]);
    await render();
    expect(rows().map(row => row.querySelector('.assistant-session-history__delegated-title')!.textContent))
      .toEqual(['Task a', 'Task b']);
    expect(rows()[0].textContent).toContain('nav.sessions.delegated.fromAccount:Bea');
    expect(rows()[0].textContent).toContain('nav.sessions.delegated.waiting:2');
    expect(rows()[0].classList.contains('is-waiting')).toBe(true);
    expect(rows()[1].classList.contains('is-waiting')).toBe(false);
    expect(rows()[1].textContent).toContain('nav.sessions.delegated.fromDevice:Laptop');

    // Review and approval happen on the AI collaboration page.
    await act(async () => (rows()[0] as HTMLButtonElement).click());
    expect(mocks.openScene).toHaveBeenCalledWith('ai-collaboration');
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
