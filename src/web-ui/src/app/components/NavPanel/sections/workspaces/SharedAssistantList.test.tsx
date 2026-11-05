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
  confirm: vi.fn(),
  remove: vi.fn(),
  supported: vi.fn(),
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
    isLocalOnlyConversation: actual.isLocalOnlyConversation,
    deleteSharedAssistantConversation: mocks.remove,
    sharedConversationDeleteSupported: mocks.supported,
  };
});
vi.mock('@/features/dispatch/sharedAssistantProblemText', () => ({
  explainSharedAssistantError: (error: unknown) => error,
}));
vi.mock('@/infrastructure/confirm-dialog', () => ({ confirmDanger: mocks.confirm }));
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
import { resetNavListMemory } from './navListMemory';

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
  for (const mock of [mocks.open, mocks.start, mocks.openSession, mocks.error, mocks.confirm, mocks.remove, mocks.supported]) {
    mock.mockReset();
  }
  mocks.confirm.mockResolvedValue(true);
  mocks.remove.mockResolvedValue(undefined);
  mocks.supported.mockResolvedValue(true);
  mocks.open.mockResolvedValue('session-b');
  mocks.start.mockResolvedValue('session-new');
  mocks.openSession.mockResolvedValue(undefined);
  mocks.peer = false;
  resetNavListMemory();
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
    // Who shares it reads beside the name; the accessible name says the same once.
    const header = document.querySelector('[data-testid="nav-shared-assistant-open"]')!;
    expect(header.querySelector('.assistant-session-history__delegated-title')!.textContent).toBe('Writer');
    expect(header.querySelector('[data-testid="nav-shared-assistant-source"]')!.textContent)
      .toBe('nav.sessions.sharedAssistant.from Ann');
    expect(header.getAttribute('aria-label')).toBe('Writer · nav.sessions.sharedAssistant.from Ann');
    // Reachable, the assistant shows the online dot.
    expect(all('nav-shared-assistant-online')).toHaveLength(1);
    expect(all('nav-shared-assistant-unavailable')).toHaveLength(0);
    expect(all('nav-shared-assistant')[0].className).not.toContain('is-unavailable');
    // Like a local assistant, the history starts closed and shows only what is viewed or waits.
    const history = document.querySelector('[data-testid="nav-shared-assistant-history"]')!;
    expect(history.getAttribute('aria-expanded')).toBe('false');
    expect(all('nav-shared-assistant-conversation').map(row => row.textContent)).toEqual(['Second draft']);
    await click(history);
    expect(history.getAttribute('aria-expanded')).toBe('true');
    const rows = all('nav-shared-assistant-conversation');
    // A row reads its title and an icon; the full sentence is its name and tooltip.
    expect(rows.map(row => row.textContent)).toEqual(['Second draft', 'First']);
    expect(rows.map(row => row.getAttribute('aria-label'))).toEqual([
      'Second draft · nav.sessions.sharedAssistant.status.approval Ann',
      'First · nav.sessions.sharedAssistant.status.queuedLimit Ann 2',
    ]);
    expect(rows.map(row => row.querySelector('[data-openbitfun-part="statusMark"]')!.getAttribute('data-status')))
      .toEqual(['approval', 'queued']);
    expect(rows[0].getAttribute('aria-current')).toBe('true');
    expect(rows[0].className).toContain('is-waiting');
    // Closed again, it stays closed after the list is shown again.
    await click(history);
    act(() => root.unmount());
    root = createRoot(container);
    await render();
    expect(all('nav-shared-assistant-conversation')).toHaveLength(1);
  });

  it('keep a closed history empty when nothing is viewed or waiting', async () => {
    mocks.jobs = { a: mocks.jobs.a };
    await render();
    expect(all('nav-shared-assistant-conversation')).toHaveLength(0);
    await click(document.querySelector('[data-testid="nav-shared-assistant-history"]')!);
    expect(all('nav-shared-assistant-conversation').map(row => row.textContent)).toEqual(['First']);
  });

  it('dim an assistant whose owner is out of reach', async () => {
    mocks.shared = [{ ...writer, online: false }];
    await render();
    expect(all('nav-shared-assistant')[0].className).toContain('is-unavailable');
    expect(all('nav-shared-assistant-online')).toHaveLength(0);
    expect(all('nav-shared-assistant-unavailable')).toHaveLength(1);
    expect(document.querySelector('[data-testid="nav-shared-assistant-open"]')!.getAttribute('aria-label'))
      .toContain('nav.sessions.sharedAssistant.offline');
    // Whether it can be reached stays a mark; the row still names who shares it.
    expect(document.querySelector('[data-testid="nav-shared-assistant-source"]')!.textContent)
      .toBe('nav.sessions.sharedAssistant.from Ann');
  });

  it('show a few conversations, the viewed one always, until asked for the rest', async () => {
    const settled = (id: string, updatedAt: number) => ({
      jobId: id, sessionId: `session-${id}`, target, title: id, state: 'succeeded', pendingPermissions: [], updatedAt,
    });
    mocks.jobs = Object.fromEntries([
      settled('c', 50), settled('d', 40), settled('e', 30), settled('f', 20),
      ['b', { ...(mocks.jobs.b as object), updatedAt: 10 }],
    ].map(entry => (Array.isArray(entry) ? entry : [entry.jobId, entry])));
    await render();
    await click(document.querySelector('[data-testid="nav-shared-assistant-history"]')!);
    const titles = () => all('nav-shared-assistant-conversation').map(row => row.querySelector('.assistant-session-history__delegated-title')!.textContent);
    // session-b is the one being viewed, so it stays in sight past the first three.
    expect(titles()).toEqual(['c', 'd', 'e', 'Second draft']);
    const more = document.querySelector('[data-testid="nav-shared-assistant-more"]')!;
    expect(more.textContent).toBe('nav.sessions.delegated.showMore 1');
    await click(more);
    expect(titles()).toEqual(['c', 'd', 'e', 'f', 'Second draft']);
  });

  it('open a conversation, the latest one, or a new one', async () => {
    await render();
    await click(document.querySelector('[data-testid="nav-shared-assistant-history"]')!);
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
    expect(document.querySelector('[data-testid="nav-shared-assistant-open"]')!.getAttribute('aria-label'))
      .toContain('nav.sessions.sharedAssistant.revoked');
    expect(all('nav-shared-assistant-unavailable')).toHaveLength(1);
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

  describe('deleting a conversation', () => {
    const settled = { jobId: 'c', sessionId: 'session-c', target, title: 'Done', state: 'succeeded', pendingPermissions: [], updatedAt: 3 };
    const openMenu = async (title: string) => {
      const history = document.querySelector('[data-testid="nav-shared-assistant-history"]')!;
      if (history.getAttribute('aria-expanded') !== 'true') await click(history);
      const row = all('nav-shared-assistant-conversation').find(item => item.textContent === title)!;
      const more = row.parentElement!.querySelector('[data-testid="nav-shared-assistant-conversation-more"]')!;
      if (more.getAttribute('aria-expanded') === 'true') await click(more);
      await click(more);
      return document.getElementById(more.getAttribute('aria-controls')!)!.querySelector('[data-menu-id="delete"]')!;
    };
    const blockedBy = (item: Element) => (item.getAttribute('aria-disabled') === 'true'
      ? item.textContent!.replace('nav.sessions.sharedAssistant.delete', '').trim()
      : null);

    it('deletes a finished one on both devices after a confirmation that names the owner', async () => {
      mocks.jobs = { ...mocks.jobs, c: settled };
      await render();
      const item = await openMenu('Done');
      expect(mocks.supported).toHaveBeenCalledWith('ann-device');
      expect(blockedBy(item)).toBeNull();
      await click(item);
      expect(mocks.confirm).toHaveBeenCalledWith(
        'nav.sessions.sharedAssistant.deleteTitle',
        'nav.sessions.sharedAssistant.deleteMessage Ann',
        { confirmText: 'nav.sessions.sharedAssistant.deleteConfirm' },
      );
      expect(mocks.remove).toHaveBeenCalledWith(settled);
    });

    it('keeps it when the confirmation is declined, and says why the owner refused', async () => {
      mocks.jobs = { ...mocks.jobs, c: settled };
      await render();
      mocks.confirm.mockResolvedValueOnce(false);
      await click(await openMenu('Done'));
      expect(mocks.remove).not.toHaveBeenCalled();

      mocks.remove.mockRejectedValueOnce(new Error('in use'));
      await click(await openMenu('Done'));
      expect(mocks.error).toHaveBeenCalledWith('nav.sessions.sharedAssistant.deleteFailed in use');
    });

    it('removes a new conversation that never reached the owner here at once, whatever the owner', async () => {
      const draft = { jobId: 'n', sessionId: 'session-n', target, title: 'New', state: 'submitting', pendingPermissions: [], updatedAt: 4 };
      mocks.shared = [{ ...writer, online: false }];
      mocks.jobs = { n: draft };
      await render();
      const item = await openMenu('New');
      expect(blockedBy(item)).toBeNull();
      expect(mocks.supported).not.toHaveBeenCalled();
      await click(item);
      expect(mocks.confirm).not.toHaveBeenCalled();
      expect(mocks.remove).toHaveBeenCalledWith(draft);
    });

    it('greys it out with the reason while it cannot be deleted', async () => {
      mocks.jobs = { ...mocks.jobs, c: settled };
      await render();
      // Waiting for approval or a slot, it must be stopped first.
      expect(blockedBy(await openMenu('Second draft'))).toBe('nav.sessions.sharedAssistant.deleteBlocked.active');
      expect(blockedBy(await openMenu('First'))).toBe('nav.sessions.sharedAssistant.deleteBlocked.active');

      mocks.supported.mockResolvedValue(false);
      expect(blockedBy(await openMenu('Done'))).toBe('nav.sessions.sharedAssistant.deleteBlocked.needsUpdate');
      mocks.supported.mockRejectedValue(new Error('unreachable'));
      expect(blockedBy(await openMenu('Done'))).toBe('nav.sessions.sharedAssistant.deleteBlocked.offline');
    });

    it('greys it out when the owner is offline or stopped sharing, without asking it', async () => {
      mocks.jobs = { c: settled };
      mocks.shared = [{ ...writer, online: false }];
      await render();
      expect(blockedBy(await openMenu('Done'))).toBe('nav.sessions.sharedAssistant.deleteBlocked.offline');
      mocks.shared = [{ ...writer, state: 'revoked' }];
      await render();
      expect(blockedBy(await openMenu('Done'))).toBe('nav.sessions.sharedAssistant.deleteBlocked.revoked');
      expect(mocks.supported).not.toHaveBeenCalled();
    });
  });
});
