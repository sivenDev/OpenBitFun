// @vitest-environment jsdom
import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import NurseryGallery from './NurseryGallery';

const mocks = vi.hoisted(() => ({ peerActive: false }));
vi.mock('@/infrastructure/runtime', () => ({ isTauriRuntime: () => true }));
vi.mock('@/infrastructure/peer-device/peerDeviceContextState', () => ({
  usePeerDeviceModeOptional: () => ({ peerMode: { active: mocks.peerActive } }),
}));
vi.mock('@/infrastructure/lan-share/lanShareStore', () => ({
  useLanShareStore: (select: (state: unknown) => unknown) => select({
    overview: null, unavailable: null, sharedWithMe: [], discovered: [],
  }),
  subscribeLanShare: () => () => {},
}));
vi.mock('@/infrastructure/contexts/WorkspaceContext', () => ({
  useWorkspaceContext: () => ({
    allAssistantWorkspacesList: [],
    openedWorkspaces: new Set(),
    assistantWorkspaces: [],
    primaryAssistantWorkspaceId: null,
    createAssistantWorkspace: vi.fn(),
    loading: false,
  }),
}));
vi.mock('@/infrastructure/i18n/hooks/useI18n', () => ({
  useI18n: () => ({ t: (key: string) => key }),
}));
vi.mock('@/shared/notification-system', () => ({ useNotification: () => ({ error: vi.fn() }) }));
vi.mock('@/shared/utils/logger', () => ({ createLogger: () => ({ error: vi.fn(), warn: vi.fn(), debug: vi.fn() }) }));
vi.mock('./AssistantCard', () => ({ default: () => null }));
vi.mock('./DiscoverPanel', () => ({
  default: ({ open }: { open: boolean }) => (open ? <div data-testid="discover-panel" /> : null),
}));

let root: Root;
let container: HTMLDivElement;
const discover = () => container.querySelector<HTMLButtonElement>('[data-testid="nursery-discover"]')!;

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal('matchMedia', () => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() }));
  container = document.createElement('div');
  document.body.append(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

describe('Discover, including adding a device by address', () => {
  it('opens on this device', async () => {
    mocks.peerActive = false;
    await act(async () => { root.render(<NurseryGallery />); });
    expect(discover().disabled).toBe(false);
    await act(async () => { discover().click(); });
    expect(container.querySelector('[data-testid="discover-panel"]')).not.toBeNull();
  });

  it('is unavailable in Peer Device Mode instead of using the controller', async () => {
    mocks.peerActive = true;
    await act(async () => { root.render(<NurseryGallery />); });
    expect(discover().disabled).toBe(true);
    await act(async () => { discover().click(); });
    expect(container.querySelector('[data-testid="discover-panel"]')).toBeNull();
  });
});
