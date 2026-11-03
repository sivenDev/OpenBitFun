// @vitest-environment jsdom
import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ConfirmDialogRenderer, useConfirmDialogStore } from '@/infrastructure/confirm-dialog';
import type { OwnedShare } from '@/infrastructure/api/service-api/LanShareAPI';
import AssistantShareManageDialog from './AssistantShareManageDialog';

const mocks = vi.hoisted(() => ({
  revoke: vi.fn(),
  unblock: vi.fn(),
  deleteTenant: vi.fn(),
  stop: vi.fn(),
  error: vi.fn(),
}));
vi.mock('@/infrastructure/api/service-api/LanShareAPI', () => ({
  lanShareAPI: {
    revoke: mocks.revoke,
    unblock: mocks.unblock,
    deleteTenant: mocks.deleteTenant,
    stop: mocks.stop,
  },
}));
vi.mock('@/infrastructure/lan-share/lanShareStore', () => ({
  useLanShareStore: (select: (state: unknown) => unknown) => select({
    refreshOverview: async () => {},
    overview: {
      status: {
        deviceId: 'me', deviceName: 'My Mac', displayName: 'Me', fingerprint: 'ZZ99ZZ',
        port: 50123, addresses: ['172.16.23.235:50123'],
      },
      shares: [],
    },
  }),
}));
vi.mock('@/infrastructure/i18n/hooks/useI18n', () => ({
  useI18n: () => ({
    t: (key: string, params?: Record<string, unknown>) =>
      params ? `${key} ${Object.values(params).join(' ')}` : key,
    formatDate: () => 'Nov 2',
  }),
}));
vi.mock('@/shared/notification-system', () => ({ useNotification: () => ({ error: mocks.error }) }));
vi.mock('@/shared/utils/logger', () => ({
  createLogger: () => ({ error: vi.fn(), warn: vi.fn(), debug: vi.fn() }),
}));

const ann = {
  publicKey: 'a'.repeat(64),
  deviceId: '0123456789abcdef0123456789abcdef',
  fingerprint: 'AB23CD',
  displayName: 'Ann',
  deviceName: "Ann's Mac",
  grantedAt: '2026-11-02T00:00:00Z',
};
const share: OwnedShare = {
  id: 'workspace',
  displayName: 'Writer',
  trust: 'full',
  sessionRetentionDays: 30,
  maxConcurrentTasks: 3,
  externalAgentUsageConfirmed: false,
  sharedAt: '2026-11-02T00:00:00Z',
  grants: [ann],
  blocked: [],
  rootPath: '/home/me/.openbitfun/personal_assistant/workspace',
  tenantDeviceIds: [ann.deviceId, 'fedcba9876543210fedcba9876543210'],
};

let root: Root;
let container: HTMLDivElement;
const onClose = vi.fn();
const onEdit = vi.fn();

const render = (current: OwnedShare = share) => act(async () => {
  root.render(
    <>
      <AssistantShareManageDialog open name="Writer" share={current} onClose={onClose} onEdit={onEdit} />
      <ConfirmDialogRenderer />
    </>,
  );
});
const buttonIn = (selector: string, text: string) =>
  [...document.querySelectorAll<HTMLButtonElement>(`${selector} button`)]
    .find((button) => button.textContent?.trim().startsWith(text))!;
const click = (button: HTMLButtonElement) => act(async () => { button.click(); });
const confirm = () => click(
  [...document.querySelectorAll<HTMLButtonElement>('[role="alertdialog"] button')]
    .find((button) => !button.textContent?.includes('dialog.confirm.cancel') && button.textContent?.trim())!,
);

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal('matchMedia', () => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() }));
  useConfirmDialogStore.setState({ isOpen: false, options: null, resolve: null });
  for (const mock of Object.values(mocks)) mock.mockReset();
  mocks.revoke.mockResolvedValue(undefined);
  mocks.unblock.mockResolvedValue(undefined);
  mocks.deleteTenant.mockResolvedValue(true);
  mocks.stop.mockResolvedValue(true);
  onClose.mockReset();
  onEdit.mockReset();
  container = document.createElement('div');
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(async () => { useConfirmDialogStore.getState().cancel(); });
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

describe('managing a shared assistant', () => {
  it('shows its settings and who uses it', async () => {
    await render();
    const body = document.querySelector('[data-testid="assistant-share-manage"]')!.textContent;
    expect(body).toContain('nursery.share.manage.summary nursery.share.dialog.trustFull 30 3');
    const users = document.querySelectorAll('[data-testid="assistant-share-user"]');
    expect(users).toHaveLength(1);
    expect(users[0].textContent).toContain('Ann');
    expect(users[0].textContent).toContain("Ann's Mac AB23CD");
    // Only the device without access offers to delete what it left.
    const left = document.querySelectorAll('[data-testid="assistant-share-left-device"]');
    expect(left).toHaveLength(1);
    expect(left[0].textContent).toContain('fedcba98');
  });

  it('removes a person after confirmation', async () => {
    await render();
    await click(buttonIn('[data-testid="assistant-share-user"]', 'nursery.share.manage.remove'));
    expect(mocks.revoke).not.toHaveBeenCalled();
    expect(document.querySelector('[role="alertdialog"]')?.textContent).toContain('nursery.share.manage.removeMessage Ann');
    await confirm();
    expect(mocks.revoke).toHaveBeenCalledWith('workspace', ann.publicKey);
  });

  it('lists blocked devices and lets one ask again without a confirmation', async () => {
    const bea = {
      publicKey: 'cd'.repeat(32),
      deviceId: 'abcdefabcdefabcdefabcdefabcdefab',
      fingerprint: 'ZX3VRN',
      displayName: 'Bea',
      deviceName: "Bea's Mac",
      blockedAt: '2026-11-03T00:00:00Z',
    };
    await render({ ...share, blocked: [bea] });
    const rows = document.querySelectorAll('[data-testid="assistant-share-blocked-device"]');
    expect(rows).toHaveLength(1);
    expect(rows[0].textContent).toContain('Bea');
    expect(rows[0].textContent).toContain("Bea's Mac ZX3VRN");
    await click(buttonIn('[data-testid="assistant-share-blocked-device"]', 'nursery.share.manage.unblock'));
    expect(document.querySelector('[role="alertdialog"]')).toBeNull();
    expect(mocks.unblock).toHaveBeenCalledWith('workspace', bea.publicKey);
  });

  it('shows no blocked section when nobody is blocked', async () => {
    await render();
    expect(document.querySelector('[data-testid="assistant-share-blocked-device"]')).toBeNull();
    expect(document.body.textContent).not.toContain('nursery.share.manage.blocked');
  });

  it('deletes what a removed device left after confirmation', async () => {
    await render();
    await click(buttonIn('[data-testid="assistant-share-left-device"]', 'nursery.share.manage.deleteData'));
    await confirm();
    expect(mocks.deleteTenant).toHaveBeenCalledWith('workspace', 'fedcba9876543210fedcba9876543210');
  });

  it('stops sharing after confirmation and closes', async () => {
    await render();
    await click(document.querySelector<HTMLButtonElement>('[data-testid="assistant-share-stop"]')!);
    expect(mocks.stop).not.toHaveBeenCalled();
    await confirm();
    expect(mocks.stop).toHaveBeenCalledWith('workspace');
    expect(onClose).toHaveBeenCalled();
  });

  it('shows the address and code people on another subnet add this device by', async () => {
    await render();
    expect(document.querySelector('[data-testid="lan-share-this-address"]')?.textContent)
      .toContain('172.16.23.235:50123');
    expect(document.querySelector('[data-testid="lan-share-this-device"]')?.textContent)
      .toContain('nursery.lanAddress.code ZZ99ZZ');
  });

  it('says nobody uses it yet and opens the settings', async () => {
    await render({ ...share, grants: [], tenantDeviceIds: [] });
    expect(document.body.textContent).toContain('nursery.share.manage.noUsers');
    expect(document.querySelector('[data-testid="assistant-share-left-device"]')).toBeNull();
    await click(buttonIn('', 'nursery.share.manage.edit'));
    expect(onEdit).toHaveBeenCalled();
  });

  it('says why a change failed', async () => {
    mocks.revoke.mockRejectedValue(new Error('gone'));
    await render();
    await click(buttonIn('[data-testid="assistant-share-user"]', 'nursery.share.manage.remove'));
    await confirm();
    expect(mocks.error).toHaveBeenCalledWith('nursery.share.manage.failed gone');
  });
});
