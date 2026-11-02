// @vitest-environment jsdom
import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ShareRequest } from '@/infrastructure/api/service-api/LanShareAPI';
import { useLanShareStore } from '@/infrastructure/lan-share/lanShareStore';
import LanShareRequestDialog from './LanShareRequestDialog';

const mocks = vi.hoisted(() => ({
  decide: vi.fn(),
  requests: vi.fn(),
  notify: vi.fn(),
  info: vi.fn(),
  error: vi.fn(),
}));
vi.mock('@/infrastructure/api/service-api/LanShareAPI', () => ({
  lanShareAPI: {
    decide: mocks.decide,
    requests: mocks.requests,
    overview: vi.fn().mockResolvedValue(null),
    sharedWithMe: vi.fn().mockResolvedValue([]),
    discovered: vi.fn().mockResolvedValue([]),
    onRequests: () => () => {},
    onShared: () => () => {},
    onSharedWithMe: () => () => {},
    onDiscovered: () => () => {},
  },
}));
vi.mock('@/infrastructure/api/service-api/SystemAPI', () => ({
  systemAPI: { sendSystemNotification: mocks.notify },
}));
vi.mock('@/infrastructure/i18n/hooks/useI18n', () => ({
  useI18n: () => ({
    t: (key: string, params?: Record<string, unknown>) =>
      params ? `${key} ${Object.values(params).join(' ')}` : key,
  }),
}));
vi.mock('@/shared/notification-system', () => ({
  useNotification: () => ({ info: mocks.info, error: mocks.error }),
}));
vi.mock('@/shared/utils/logger', () => ({
  createLogger: () => ({ error: vi.fn(), warn: vi.fn(), debug: vi.fn() }),
}));

const NOW = 1_800_000_000_000;

function request(id: string, name: string, secondsLeft = 60): ShareRequest {
  return {
    requestId: id,
    assistantId: 'workspace',
    assistantName: 'Writer',
    peer: {
      publicKey: id.repeat(64).slice(0, 64),
      deviceId: '0123456789abcdef0123456789abcdef',
      fingerprint: 'AB23CD',
      displayName: name,
      deviceName: `${name}'s Mac`,
    },
    openedAtMs: NOW,
    deadlineMs: NOW + secondsLeft * 1000,
  };
}

let root: Root;
let container: HTMLDivElement;
const render = () => act(async () => { root.render(<LanShareRequestDialog />); });
const setRequests = (requests: ShareRequest[]) =>
  act(async () => { useLanShareStore.setState({ requests }); });
const button = (part: string) =>
  document.querySelector<HTMLButtonElement>(`[data-testid="lan-share-request-${part}"]`)!;
const click = (part: string) => act(async () => { button(part).click(); });

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal('matchMedia', () => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() }));
  vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval', 'Date'] });
  vi.setSystemTime(NOW);
  for (const mock of Object.values(mocks)) mock.mockReset();
  mocks.decide.mockResolvedValue(undefined);
  mocks.requests.mockResolvedValue([]);
  mocks.notify.mockResolvedValue(undefined);
  useLanShareStore.setState({ requests: [] });
  vi.spyOn(document, 'hasFocus').mockReturnValue(true);
  container = document.createElement('div');
  document.body.append(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('confirming a request to use a shared assistant', () => {
  it('counts down to the deadline and allows', async () => {
    await render();
    expect(document.querySelector('[data-testid="lan-share-request"]')).toBeNull();
    await setRequests([request('a', 'Ann')]);
    expect(document.body.textContent).toContain("nursery.share.request.message Ann Ann's Mac Writer");
    expect(document.querySelector('[data-testid="lan-share-request-code"]')!.textContent).toContain('AB23CD');
    expect(button('allow').textContent).toContain('nursery.share.request.allow 60');

    await act(async () => { vi.advanceTimersByTime(15_000); });
    expect(button('allow').textContent).toContain('nursery.share.request.allow 45');

    mocks.requests.mockResolvedValue([]);
    await click('allow');
    expect(mocks.decide).toHaveBeenCalledWith('a', 'allow');
  });

  it('asks about queued requests one at a time', async () => {
    await render();
    await setRequests([request('a', 'Ann'), request('b', 'Bo', 50)]);
    expect(document.body.textContent).toContain('nursery.share.request.queued 1');
    mocks.requests.mockResolvedValue([request('b', 'Bo', 50)]);
    await click('decline');
    expect(mocks.decide).toHaveBeenCalledWith('a', 'decline');
    expect(document.body.textContent).toContain("Bo's Mac");
    expect(document.body.textContent).not.toContain('nursery.share.request.queued');
    expect(button('allow').textContent).toContain('nursery.share.request.allow 50');

    await click('block');
    expect(mocks.decide).toHaveBeenCalledWith('b', 'block');
  });

  it('cannot be answered once the time is up, and says it was declined', async () => {
    await render();
    await setRequests([request('a', 'Ann', 2)]);
    await act(async () => { vi.advanceTimersByTime(2_000); });
    expect(button('allow').textContent).toContain('nursery.share.request.allow 0');
    expect(button('allow').disabled).toBe(true);
    expect(button('decline').disabled).toBe(true);
    // The host declines it and drops it from the queue.
    await setRequests([]);
    expect(document.querySelector('[data-testid="lan-share-request"]')).toBeNull();
    expect(mocks.info).toHaveBeenCalledWith('nursery.share.request.expired Ann');
    expect(mocks.decide).not.toHaveBeenCalled();
  });

  it('notifies the system when the window is in the background', async () => {
    vi.mocked(document.hasFocus).mockReturnValue(false);
    await render();
    await setRequests([request('a', 'Ann')]);
    expect(mocks.notify).toHaveBeenCalledWith(
      'nursery.share.request.title',
      'nursery.share.request.notificationBody Ann Writer',
    );
    await setRequests([request('a', 'Ann')]);
    expect(mocks.notify).toHaveBeenCalledTimes(1);
  });
});
