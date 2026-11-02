import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { LanShareOverview, OwnedShare } from '@/infrastructure/api/service-api/LanShareAPI';

const mocks = vi.hoisted(() => ({
  listeners: new Map<string, (payload?: unknown) => void>(),
  released: vi.fn(),
  overview: vi.fn(),
  requests: vi.fn(),
  sharedWithMe: vi.fn(),
  discovered: vi.fn(),
}));
vi.mock('@/infrastructure/api/service-api/LanShareAPI', () => {
  const on = (event: string) => (handler: (payload?: unknown) => void) => {
    mocks.listeners.set(event, handler);
    return mocks.released;
  };
  return {
    lanShareAPI: {
      overview: mocks.overview,
      requests: mocks.requests,
      sharedWithMe: mocks.sharedWithMe,
      discovered: mocks.discovered,
      onRequests: on('requests'),
      onShared: on('shared'),
      onSharedWithMe: on('sharedWithMe'),
      onDiscovered: on('discovered'),
    },
  };
});
vi.mock('@/shared/utils/logger', () => ({
  createLogger: () => ({ error: vi.fn(), warn: vi.fn(), debug: vi.fn() }),
}));

import {
  assistantShareId,
  secondsLeft,
  shareForWorkspace,
  subscribeLanShare,
  useLanShareStore,
} from './lanShareStore';

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

beforeEach(() => {
  mocks.listeners.clear();
  for (const mock of [mocks.released, mocks.overview, mocks.requests, mocks.sharedWithMe, mocks.discovered]) {
    mock.mockReset();
  }
  mocks.overview.mockResolvedValue({ status: {}, shares: [] });
  mocks.requests.mockResolvedValue([]);
  mocks.sharedWithMe.mockResolvedValue([]);
  mocks.discovered.mockResolvedValue([]);
  useLanShareStore.setState(useLanShareStore.getInitialState(), true);
});

describe('LAN sharing store', () => {
  it('names a share by the assistant directory and finds it by workspace', () => {
    expect(assistantShareId('/home/me/.openbitfun/personal_assistant/workspace-notes')).toBe('workspace-notes');
    expect(assistantShareId('C:\\Users\\me\\.openbitfun\\personal_assistant\\workspace\\')).toBe('workspace');
    const share = { id: 'workspace-notes', rootPath: '/elsewhere/workspace-notes' } as OwnedShare;
    const overview = { shares: [share] } as unknown as LanShareOverview;
    expect(shareForWorkspace(overview, '/home/me/.openbitfun/personal_assistant/workspace-notes')).toBe(share);
    expect(shareForWorkspace(overview, '/home/me/.openbitfun/personal_assistant/workspace')).toBeUndefined();
    expect(shareForWorkspace(null, '/x/workspace-notes')).toBeUndefined();
  });

  it('counts whole seconds left and never below zero', () => {
    expect(secondsLeft(10_000, 0)).toBe(10);
    expect(secondsLeft(10_000, 9_001)).toBe(1);
    expect(secondsLeft(10_000, 12_000)).toBe(0);
  });

  it('listens once for all views and follows the host events', async () => {
    const first = subscribeLanShare();
    const second = subscribeLanShare();
    await flush();
    expect(mocks.overview).toHaveBeenCalledTimes(1);
    expect(mocks.listeners.size).toBe(4);

    const request = { requestId: 'r1' };
    mocks.listeners.get('requests')!({ requests: [request] });
    expect(useLanShareStore.getState().requests).toEqual([request]);
    mocks.listeners.get('sharedWithMe')!();
    await flush();
    expect(mocks.sharedWithMe).toHaveBeenCalledTimes(2);

    first();
    first();
    expect(mocks.released).not.toHaveBeenCalled();
    second();
    expect(mocks.released).toHaveBeenCalledTimes(4);
  });

  it('reports why sharing is unavailable', async () => {
    mocks.overview.mockRejectedValue(new Error('sharing on the local network is still starting'));
    await useLanShareStore.getState().refreshOverview();
    expect(useLanShareStore.getState().unavailable).toBe('sharing on the local network is still starting');
    expect(useLanShareStore.getState().overview).toBeNull();
  });
});
