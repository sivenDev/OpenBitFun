// @vitest-environment jsdom
import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ConfirmDialogRenderer, useConfirmDialogStore } from '@/infrastructure/confirm-dialog';
import type { SharedWithMe } from '@/infrastructure/api/service-api/LanShareAPI';
import SharedAssistantCard from './SharedAssistantCard';

const mocks = vi.hoisted(() => ({ remove: vi.fn(), sharedWithMe: vi.fn(), error: vi.fn() }));
vi.mock('@/infrastructure/api/service-api/LanShareAPI', () => ({
  lanShareAPI: { remove: mocks.remove, sharedWithMe: mocks.sharedWithMe },
}));
vi.mock('@/app/components/AssistantAvatar', () => ({ AssistantAvatar: () => null }));
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

const shared: SharedWithMe = {
  owner: {
    publicKey: 'a'.repeat(64),
    deviceId: '0123456789abcdef0123456789abcdef',
    fingerprint: 'AB23CD',
    displayName: 'Ann',
    deviceName: "Ann's Mac",
  },
  assistantId: 'workspace',
  name: 'Writer',
  description: 'Drafts and edits',
  grantedAt: '2026-11-02T00:00:00Z',
  state: 'available',
};

let root: Root;
let container: HTMLDivElement;
const render = (assistant: SharedWithMe = shared) => act(async () => {
  root.render(<><SharedAssistantCard assistant={assistant} /><ConfirmDialogRenderer /></>);
});
const click = (element: Element) => act(async () => { (element as HTMLElement).click(); });
const confirm = () => click(
  [...document.querySelectorAll<HTMLButtonElement>('[role="alertdialog"] button')]
    .find((button) => button.textContent?.trim() === 'nursery.sharedWithMe.remove')!,
);

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal('matchMedia', () => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() }));
  useConfirmDialogStore.setState({ isOpen: false, options: null, resolve: null });
  for (const mock of Object.values(mocks)) mock.mockReset();
  mocks.remove.mockResolvedValue(undefined);
  mocks.sharedWithMe.mockResolvedValue([]);
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

describe('an assistant shared with this device', () => {
  it('is marked with where it comes from and shows the details', async () => {
    await render();
    const card = document.querySelector('[data-testid="shared-assistant-card"]')!;
    expect(card.textContent).toContain('Writer');
    expect(card.textContent).toContain('nursery.sharedWithMe.badge');
    expect(card.textContent).toContain('nursery.sharedWithMe.from Ann');
    await click(card.querySelector('button.assistant-card__main')!);
    const details = document.querySelector('[data-testid="shared-assistant-details"]')!.textContent;
    expect(details).toContain('Drafts and edits');
    expect(details).toContain("nursery.sharedWithMe.source Ann Ann's Mac AB23CD");
    expect(details).toContain('nursery.discover.privacy');
  });

  it('is marked when the owner stopped sharing it', async () => {
    await render({ ...shared, state: 'revoked' });
    const card = document.querySelector('[data-testid="shared-assistant-card"]')!;
    expect(card.className).toContain('shared-assistant-card--revoked');
    expect(card.textContent).toContain('nursery.sharedWithMe.revoked');
    await click(card.querySelector('button.assistant-card__main')!);
    expect(document.body.textContent).toContain('nursery.sharedWithMe.revokedHint');
  });

  it('is removed after confirmation', async () => {
    await render();
    await click(document.querySelector('[data-testid="shared-assistant-remove"]')!);
    expect(mocks.remove).not.toHaveBeenCalled();
    expect(document.querySelector('[role="alertdialog"]')!.textContent).toContain('nursery.sharedWithMe.removeMessage Ann');
    await confirm();
    expect(mocks.remove).toHaveBeenCalledWith(shared.owner.publicKey, 'workspace');
    expect(mocks.sharedWithMe).toHaveBeenCalled();
  });

  it('says why it could not be removed', async () => {
    mocks.remove.mockRejectedValue(new Error('disk full'));
    await render();
    await click(document.querySelector('[data-testid="shared-assistant-remove"]')!);
    await confirm();
    expect(mocks.error).toHaveBeenCalledWith('nursery.sharedWithMe.removeFailed disk full');
  });
});
