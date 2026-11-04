// @vitest-environment jsdom
import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SharedWithMe } from '@/infrastructure/api/service-api/LanShareAPI';

const mocks = vi.hoisted(() => ({
  open: vi.fn(),
  start: vi.fn(),
  openSession: vi.fn(),
  error: vi.fn(),
  peer: false,
  shared: [] as unknown[],
  jobs: {} as Record<string, unknown>,
}));
vi.mock('@/features/dispatch/sharedAssistantConversation', async () => {
  const actual = await vi.importActual<Record<string, unknown>>('@/features/dispatch/sharedAssistantConversation');
  return {
    sharedAssistantConversations: actual.sharedAssistantConversations,
    openSharedAssistantConversation: mocks.open,
    openSharedAssistantConversationJob: actual.openSharedAssistantConversationJob,
    startSharedAssistantConversation: mocks.start,
  };
});
vi.mock('@/flow_chat/services/FlowChatManager', () => ({ flowChatManager: {} }));
vi.mock('@/flow_chat/services/sessionActivation', () => ({ openMainSession: mocks.openSession }));
vi.mock('@/flow_chat/store/FlowChatStore', () => {
  const state = { sessions: new Map([['session-b', { title: 'Second draft' }]]), activeSessionId: 'session-b' };
  return { flowChatStore: { subscribe: () => () => {}, getState: () => state } };
});
vi.mock('@/features/dispatch/dispatchJobStore', () => ({
  dispatchJobStore: (select: (state: { jobs: Record<string, unknown> }) => unknown) => select({ jobs: mocks.jobs }),
}));
vi.mock('@/infrastructure/lan-share/lanShareStore', () => ({
  useLanShareStore: (select: (state: { sharedWithMe: unknown[] }) => unknown) => select({ sharedWithMe: mocks.shared }),
}));
vi.mock('@/infrastructure/peer-device/peerDeviceContextState', () => ({
  usePeerDeviceModeOptional: () => ({ peerMode: { active: mocks.peer } }),
}));
vi.mock('@/app/stores/sceneStore', () => ({
  useSceneStore: (select: (state: { activeTabId: string }) => unknown) => select({ activeTabId: 'session' }),
}));
vi.mock('@/infrastructure/i18n', () => ({
  useI18n: () => ({
    t: (key: string, params?: Record<string, unknown>) => (params ? `${key} ${Object.values(params).join(' ')}` : key),
  }),
}));
vi.mock('@/shared/notification-system', () => ({ notificationService: { error: mocks.error } }));
vi.mock('@/shared/utils/logger', () => ({
  createLogger: () => ({ error: vi.fn(), warn: vi.fn(), info: vi.fn(), debug: vi.fn() }),
}));

import SharedAssistantList from './SharedAssistantList';

const writer: SharedWithMe = {
  owner: { publicKey: 'a'.repeat(64), deviceId: 'ann-device', fingerprint: 'AB23CD', displayName: 'Ann', deviceName: "Ann's Mac" },
  assistantId: 'writer',
  name: 'Writer',
  grantedAt: '2026-11-03T00:00:00Z',
  state: 'available',
  online: true,
};
const target = { kind: 'sharedAssistant', ownerDeviceId: 'ann-device', assistantId: 'writer', displayName: 'Writer' };
const ref = { ownerDeviceId: 'ann-device', assistantId: 'writer', name: 'Writer' };

let root: Root;
let container: HTMLDivElement;
const render = () => act(async () => { root.render(<SharedAssistantList />); });
const click = (element: Element) => act(async () => { (element as HTMLElement).click(); });
const all = (testId: string) => [...document.querySelectorAll(`[data-testid="${testId}"]`)];

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  for (const mock of [mocks.open, mocks.start, mocks.openSession, mocks.error]) mock.mockReset();
  mocks.open.mockResolvedValue('session-b');
  mocks.start.mockResolvedValue('session-new');
  mocks.openSession.mockResolvedValue(undefined);
  mocks.peer = false;
  mocks.shared = [writer];
  mocks.jobs = {
    a: { jobId: 'a', sessionId: 'session-a', target, title: 'First', state: 'queued', pendingPermissions: [], hold: { reason: 'shareLimit', limit: 2, sinceMs: 1 }, updatedAt: 1 },
    b: { jobId: 'b', sessionId: 'session-b', target, title: 'Second', state: 'running', pendingPermissions: [{ requestId: 'p' }], updatedAt: 2 },
  };
  container = document.createElement('div');
  document.body.append(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('the assistants shared with this device in the navigation', () => {
  it('list each assistant with its conversations, newest first, and what they wait on', async () => {
    await render();
    expect(all('nav-shared-assistant')).toHaveLength(1);
    expect(document.querySelector('[data-testid="nav-shared-assistant-open"]')!.textContent)
      .toContain('nav.sessions.sharedAssistant.from Ann');
    const rows = all('nav-shared-assistant-conversation');
    expect(rows.map(row => row.textContent)).toEqual([
      'Second draftnav.sessions.sharedAssistant.status.approval Ann',
      'Firstnav.sessions.sharedAssistant.status.queuedLimit Ann 2',
    ]);
    expect(rows[0].getAttribute('aria-current')).toBe('true');
    expect(rows[0].className).toContain('is-waiting');
  });

  it('open a conversation, the latest one, or a new one', async () => {
    await render();
    await click(all('nav-shared-assistant-conversation')[1]);
    expect(mocks.openSession).toHaveBeenCalledWith('session-a');
    await click(document.querySelector('[data-testid="nav-shared-assistant-open"]')!);
    expect(mocks.open).toHaveBeenCalledWith(ref, true);
    await click(document.querySelector('[data-testid="nav-shared-assistant-new"]')!);
    expect(mocks.start).toHaveBeenCalledWith(ref);
  });

  it('say why a conversation did not open', async () => {
    mocks.start.mockRejectedValue(new Error('owner offline'));
    await render();
    await click(document.querySelector('[data-testid="nav-shared-assistant-new"]')!);
    expect(mocks.error).toHaveBeenCalledWith('nav.sessions.sharedAssistant.openFailed owner offline');
  });

  it('mark an assistant no longer shared, which takes no new conversation', async () => {
    mocks.shared = [{ ...writer, state: 'revoked' }];
    await render();
    expect(document.querySelector('[data-testid="nav-shared-assistant-new"]')).toBeNull();
    expect(document.querySelector('[data-testid="nav-shared-assistant-open"]')!.textContent)
      .toContain('nav.sessions.sharedAssistant.revoked');
    await click(document.querySelector('[data-testid="nav-shared-assistant-open"]')!);
    expect(mocks.open).toHaveBeenCalledWith(ref, false);
  });

  it('list nothing in Peer Device Mode or when nothing is shared', async () => {
    mocks.peer = true;
    await render();
    expect(all('nav-shared-assistant')).toHaveLength(0);
    mocks.peer = false;
    mocks.shared = [];
    await render();
    expect(document.querySelector('[data-testid="nav-shared-assistants"]')).toBeNull();
  });
});
