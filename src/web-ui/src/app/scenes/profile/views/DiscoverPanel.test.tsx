// @vitest-environment jsdom
import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DiscoveredDevice, ShareAccess } from '@/infrastructure/api/service-api/LanShareAPI';
import { useLanShareStore } from '@/infrastructure/lan-share/lanShareStore';
import DiscoverPanel from './DiscoverPanel';

const mocks = vi.hoisted(() => ({
  discoverStart: vi.fn(),
  discoverStop: vi.fn(),
  discovered: vi.fn(),
  request: vi.fn(),
  setDisplayName: vi.fn(),
  overview: vi.fn(),
  openNetworkSettings: vi.fn(),
  success: vi.fn(),
  info: vi.fn(),
  error: vi.fn(),
}));
vi.mock('@/infrastructure/api/service-api/LanShareAPI', () => ({
  lanShareAPI: {
    discoverStart: mocks.discoverStart,
    discoverStop: mocks.discoverStop,
    discovered: mocks.discovered,
    request: mocks.request,
    setDisplayName: mocks.setDisplayName,
    overview: mocks.overview,
    openNetworkSettings: mocks.openNetworkSettings,
  },
}));
vi.mock('@/app/components/AssistantAvatar', () => ({ AssistantAvatar: () => null }));
vi.mock('@/infrastructure/i18n/hooks/useI18n', () => ({
  useI18n: () => ({
    t: (key: string, params?: Record<string, unknown>) =>
      params ? `${key} ${Object.values(params).join(' ')}` : key,
  }),
}));
vi.mock('@/shared/notification-system', () => ({
  useNotification: () => ({ success: mocks.success, info: mocks.info, error: mocks.error }),
}));
vi.mock('@/shared/utils/logger', () => ({
  createLogger: () => ({ error: vi.fn(), warn: vi.fn(), debug: vi.fn() }),
}));

const NOW = 1_800_000_000_000;

function device(
  name: string,
  access: ShareAccess[],
  status: DiscoveredDevice['status'] = { state: 'ready' },
): DiscoveredDevice {
  return {
    deviceId: `${name}-device`,
    deviceName: `${name}'s Mac`,
    displayName: name,
    fingerprint: 'AB23CD',
    publicKey: `${name}-key`,
    status,
    assistants: access.map((state, i) => ({ id: `workspace-${i}`, name: `${name} helper ${i}`, access: state })),
  };
}

let root: Root;
let container: HTMLDivElement;
const onClose = vi.fn();
const render = () => act(async () => { root.render(<DiscoverPanel open onClose={onClose} />); });
const setDevices = (discovered: DiscoveredDevice[]) =>
  act(async () => { useLanShareStore.setState({ discovered }); });
const tiles = () => [...document.querySelectorAll<HTMLButtonElement>('[data-testid="discover-assistant"]')];
const click = (element: HTMLElement) => act(async () => { element.click(); });
const text = () => document.querySelector('[data-testid="discover-panel"]')!.textContent ?? '';

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal('matchMedia', () => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() }));
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'Date'] });
  vi.setSystemTime(NOW);
  for (const mock of Object.values(mocks)) mock.mockReset();
  mocks.discoverStart.mockResolvedValue([]);
  mocks.discoverStop.mockResolvedValue(undefined);
  mocks.discovered.mockResolvedValue([]);
  mocks.overview.mockResolvedValue(null);
  mocks.openNetworkSettings.mockResolvedValue(undefined);
  useLanShareStore.setState({
    discovered: [],
    overview: {
      status: { deviceId: 'me', deviceName: 'My Mac', displayName: 'Me', fingerprint: 'ZZ99ZZ' },
      shares: [],
    },
  });
  onClose.mockReset();
  container = document.createElement('div');
  document.body.append(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('discovering shared assistants', () => {
  it('looks while open, then says why nothing was found and opens the settings', async () => {
    await render();
    expect(mocks.discoverStart).toHaveBeenCalledTimes(1);
    expect(text()).toContain('nursery.discover.searching');
    await act(async () => { vi.advanceTimersByTime(4_000); });
    expect(text()).toContain('nursery.discover.emptyTitle');
    expect(text()).toContain('nursery.discover.emptyHint');
    await click(document.querySelector<HTMLButtonElement>('[data-testid="discover-open-settings"]')!);
    expect(mocks.openNetworkSettings).toHaveBeenCalled();
    act(() => root.unmount());
    expect(mocks.discoverStop).toHaveBeenCalled();
    root = createRoot(container);
  });

  it('says why devices cannot be looked for', async () => {
    mocks.discoverStart.mockRejectedValue(new Error('cannot look for devices on the local network: denied'));
    await render();
    expect(text()).toContain('nursery.discover.problem');
    expect(text()).toContain('cannot look for devices on the local network: denied');
    expect(document.querySelectorAll('[data-testid="discover-open-settings"]')).toHaveLength(1);
  });

  it('lists assistants by device with where each request stands', async () => {
    await render();
    await setDevices([
      device('Ann', [
        { state: 'available' },
        { state: 'pending', requestId: 'r1', deadlineMs: NOW + 42_000 },
        { state: 'granted' },
        { state: 'cooling', retryAtMs: NOW + 30_000 },
      ]),
      device('Bo', [{ state: 'available' }], { state: 'needsUpdate', message: 'protocol 2' }),
      device('Cy', [], { state: 'unreachable', message: 'timed out' }),
    ]);
    expect(document.querySelectorAll('[data-testid="discover-device"]')).toHaveLength(3);
    expect(text()).toContain("nursery.discover.device Ann's Mac AB23CD");
    const labels = tiles().map((tile) => tile.textContent);
    expect(labels[0]).toContain('nursery.discover.ask');
    expect(labels[1]).toContain('nursery.discover.pending 42');
    expect(labels[2]).toContain('nursery.discover.added');
    expect(labels[3]).toContain('nursery.discover.cooling 30');
    expect(tiles().map((tile) => tile.disabled)).toEqual([false, true, true, true, true]);
    expect(text()).toContain('nursery.discover.needsUpdate');
    expect(text()).toContain('nursery.discover.unreachable');

    await act(async () => { vi.advanceTimersByTime(2_000); });
    expect(tiles()[1].textContent).toContain('nursery.discover.pending 40');
  });

  it('asks for an assistant and says how it ended', async () => {
    await render();
    await setDevices([device('Ann', [{ state: 'available' }, { state: 'available' }])]);
    mocks.request.mockResolvedValue({ requestId: 'r1', deadlineMs: NOW + 60_000 });
    await click(tiles()[0]);
    expect(mocks.request).toHaveBeenCalledWith('Ann-device', 'workspace-0');

    await setDevices([device('Ann', [
      { state: 'pending', requestId: 'r1', deadlineMs: NOW + 60_000 },
      { state: 'pending', requestId: 'r2', deadlineMs: NOW + 60_000 },
    ])]);
    await setDevices([device('Ann', [{ state: 'granted' }, { state: 'cooling', retryAtMs: NOW + 60_000 }])]);
    expect(mocks.success).toHaveBeenCalledWith('nursery.discover.granted Ann helper 0');
    expect(mocks.info).toHaveBeenCalledWith('nursery.discover.declined Ann helper 1');
  });

  it('reports a request that was answered at once or not sent', async () => {
    const ann = device('Ann', [{ state: 'available' }]);
    mocks.discovered.mockResolvedValue([ann]);
    await render();
    await setDevices([ann]);
    mocks.request.mockResolvedValueOnce({ requestId: 'r1', deadlineMs: NOW, granted: false });
    await click(tiles()[0]);
    expect(mocks.info).toHaveBeenCalledWith('nursery.discover.declined Ann helper 0');

    mocks.request.mockRejectedValueOnce(new Error('that device is no longer on the local network'));
    await click(tiles()[0]);
    expect(mocks.error).toHaveBeenCalledWith(
      'nursery.discover.requestFailed that device is no longer on the local network',
    );
  });

  it('shows the privacy note and changes the name others see', async () => {
    mocks.setDisplayName.mockResolvedValue({});
    await render();
    expect(document.body.textContent).toContain('nursery.discover.privacy');
    expect(document.body.textContent).toContain('nursery.discover.shownAs Me');
    await click(document.querySelector<HTMLButtonElement>('[data-testid="discover-name-edit"]')!);
    const input = document.querySelector<HTMLInputElement>('[data-testid="discover-name-input"]')!;
    expect(input.value).toBe('Me');
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
      setter.call(input, 'Ann at work');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await act(async () => {
      input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    });
    expect(mocks.setDisplayName).toHaveBeenCalledWith('Ann at work');
  });
});
