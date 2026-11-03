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
  probeAddress: vi.fn(),
  addDevice: vi.fn(),
  removeDevice: vi.fn(),
  setDeviceAddress: vi.fn(),
  confirmDanger: vi.fn(),
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
    probeAddress: mocks.probeAddress,
    addDevice: mocks.addDevice,
    removeDevice: mocks.removeDevice,
    setDeviceAddress: mocks.setDeviceAddress,
  },
  shownAddress: (address: string) => address.replace(/:47652$/, ''),
}));
vi.mock('@/infrastructure/confirm-dialog', () => ({ confirmDanger: mocks.confirmDanger }));
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
      status: { deviceId: 'me', deviceName: 'My Mac', displayName: 'Me', fingerprint: 'ZZ99ZZ', addresses: [] },
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

  it('lets a device ask again once its cooldown has passed, before the catalog is read again', async () => {
    await render();
    await setDevices([device('Ann', [{ state: 'cooling', retryAtMs: NOW + 2_000 }])]);
    expect(tiles()[0].disabled).toBe(true);
    expect(tiles()[0].textContent).toContain('nursery.discover.cooling 2');

    await act(async () => { vi.advanceTimersByTime(3_000); });
    expect(tiles()[0].disabled).toBe(false);
    expect(tiles()[0].textContent).toContain('nursery.discover.ask');
    mocks.request.mockResolvedValue({ requestId: 'r2', deadlineMs: NOW + 63_000 });
    await click(tiles()[0]);
    expect(mocks.request).toHaveBeenCalledWith('Ann-device', 'workspace-0');
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

    // Said in the panel: a toast would sit under the dialog.
    mocks.request.mockRejectedValueOnce(new Error('the owner is busy'));
    await click(tiles()[0]);
    expect(document.querySelector('[data-testid="discover-request-problem"]')?.textContent)
      .toContain('nursery.discover.requestFailed the owner is busy');
    expect(mocks.error).not.toHaveBeenCalled();

    // An owner that went away is said under its name, once.
    mocks.discovered.mockResolvedValue([
      device('Ann', [], { state: 'unreachable', message: 'not on the local network' }),
    ]);
    mocks.request.mockRejectedValueOnce(new Error('the device sharing this assistant is not on the local network'));
    await click(tiles()[0]);
    expect(document.querySelector('[data-testid="discover-request-problem"]')).toBeNull();
    expect(document.querySelector('[data-testid="discover-device-note"]')?.textContent)
      .toBe('nursery.discover.unreachable');
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

describe('devices on another subnet', () => {
  const found = {
    address: '172.17.9.12:47652',
    deviceId: 'bea-device',
    deviceName: "Bea's Mac",
    displayName: 'Bea',
    fingerprint: 'QW45ER',
    publicKey: 'bea-key',
    assistants: 2,
    added: false,
  };
  const byId = (id: string) => document.querySelector<HTMLElement>(`[data-testid="${id}"]`);
  const type = (id: string, value: string) => act(async () => {
    const input = byId(id) as HTMLInputElement;
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
    setter.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });

  it('shows where this device can be added while it shares', async () => {
    useLanShareStore.setState({
      overview: {
        status: {
          deviceId: 'me', deviceName: 'My Mac', displayName: 'Me', fingerprint: 'ZZ99ZZ',
          port: 47652, addresses: ['172.16.23.235'],
        },
        shares: [],
      },
    });
    await render();
    expect(byId('lan-share-this-address')?.textContent).toContain('172.16.23.235');
    expect(byId('lan-share-this-device')?.textContent).toContain('ZZ99ZZ');
  });

  it('adds a device by address only after its code was shown', async () => {
    mocks.probeAddress.mockResolvedValue({ state: 'reached', device: found });
    mocks.addDevice.mockResolvedValue({ state: 'reached', device: { ...found, added: true } });
    await render();
    expect(byId('lan-share-this-device')).toBeNull();
    await click(byId('discover-by-address-open')!);
    await type('discover-by-address-input', '172.17.9.12');
    await click(byId('discover-by-address-look')!);
    expect(mocks.probeAddress).toHaveBeenCalledWith('172.17.9.12');
    expect(mocks.addDevice).not.toHaveBeenCalled();
    expect(byId('discover-by-address-code')?.textContent).toContain('QW45ER');
    expect(byId('discover-by-address-found')?.textContent).toContain('nursery.discover.byAddress.sharing 2');

    await click(byId('discover-by-address-add')!);
    expect(mocks.addDevice).toHaveBeenCalledWith('172.17.9.12:47652', 'bea-key');
    expect(mocks.success).toHaveBeenCalledWith('nursery.discover.byAddress.added Bea');
    expect(byId('discover-by-address')).toBeNull();
    expect(mocks.discovered).toHaveBeenCalled();
  });

  it('says in one sentence why an address cannot be used', async () => {
    mocks.probeAddress.mockResolvedValue({ state: 'problem', problem: 'unreachable', message: 'connect refused' });
    await render();
    await click(byId('discover-by-address-open')!);
    await type('discover-by-address-input', '172.17.9.99');
    await click(byId('discover-by-address-look')!);
    expect(byId('discover-by-address-problem')?.textContent)
      .toContain('nursery.discover.byAddress.problems.unreachable');
    expect(byId('discover-by-address-found')).toBeNull();
  });

  it('lets a device added by address move or go, and says when it changed', async () => {
    mocks.confirmDanger.mockResolvedValue(true);
    mocks.removeDevice.mockResolvedValue(true);
    mocks.setDeviceAddress.mockResolvedValue({ state: 'problem', problem: 'identityChanged', message: 'key' });
    await render();
    await setDevices([{
      ...device('Bea', [], { state: 'identityChanged', message: 'key' }),
      addedAddress: '172.17.9.12:47652',
    }]);
    expect(byId('discover-device-added')?.textContent).toContain('nursery.discover.addedBadge 172.17.9.12');
    expect(byId('discover-device-note')?.textContent).toBe('nursery.discover.identityChanged');

    await click(byId('discover-device-move')!);
    expect((byId('discover-device-address-input') as HTMLInputElement).value).toBe('172.17.9.12');
    await type('discover-device-address-input', '172.17.9.13');
    await act(async () => {
      byId('discover-device-address-input')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    });
    expect(mocks.setDeviceAddress).toHaveBeenCalledWith('Bea-device', '172.17.9.13');
    expect(byId('discover-device-address-problem')?.textContent)
      .toContain('nursery.discover.byAddress.problems.identityChanged');

    await click(byId('discover-device-remove')!);
    expect(mocks.confirmDanger).toHaveBeenCalled();
    expect(mocks.removeDevice).toHaveBeenCalledWith('Bea-device');
  });
});
