// @vitest-environment jsdom
import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { OwnedShare } from '@/infrastructure/api/service-api/LanShareAPI';
import AssistantShareDialog from './AssistantShareDialog';

const mocks = vi.hoisted(() => ({
  share: vi.fn(),
  update: vi.fn(),
  clients: vi.fn(),
  success: vi.fn(),
  error: vi.fn(),
}));
vi.mock('@/infrastructure/api/service-api/LanShareAPI', () => ({
  lanShareAPI: { share: mocks.share, update: mocks.update },
}));
vi.mock('@/infrastructure/api/service-api/ACPClientAPI', () => ({
  ACPClientAPI: { getClients: mocks.clients },
}));
vi.mock('@/infrastructure/lan-share/lanShareStore', () => ({
  useLanShareStore: (select: (state: { refreshOverview: () => Promise<void> }) => unknown) =>
    select({ refreshOverview: async () => {} }),
}));
vi.mock('@/infrastructure/i18n/hooks/useI18n', () => ({
  useI18n: () => ({ t: (key: string) => key }),
}));
vi.mock('@/shared/notification-system', () => ({
  useNotification: () => ({ success: mocks.success, error: mocks.error }),
}));
vi.mock('@/shared/utils/logger', () => ({
  createLogger: () => ({ error: vi.fn(), warn: vi.fn(), debug: vi.fn() }),
}));

let root: Root;
let container: HTMLDivElement;
const onClose = vi.fn();

const render = (share?: OwnedShare) => act(async () => {
  root.render(
    <AssistantShareDialog open assistantId="workspace" name="Writer" share={share} onClose={onClose} />,
  );
});
const flush = () => act(async () => { await Promise.resolve(); });
const byText = (text: string) =>
  [...document.querySelectorAll<HTMLElement>('button, label, [role="radio"]')]
    .find((element) => element.textContent?.trim() === text);
const submit = () => document.querySelector<HTMLButtonElement>('[data-testid="assistant-share-submit"]')!;
const click = (element: HTMLElement | undefined) => act(async () => { element!.click(); });

const existing: OwnedShare = {
  id: 'workspace',
  displayName: 'Writer',
  trust: 'normal',
  sessionRetentionDays: 30,
  maxConcurrentTasks: 4,
  externalAgentUsageConfirmed: true,
  sharedAt: '2026-11-02T00:00:00Z',
  grants: [],
  blocked: [],
  rootPath: '/home/me/.openbitfun/personal_assistant/workspace',
  tenantDeviceIds: [],
};

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal('matchMedia', () => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() }));
  mocks.share.mockReset().mockResolvedValue({});
  mocks.update.mockReset().mockResolvedValue({});
  mocks.clients.mockReset().mockResolvedValue([]);
  mocks.success.mockReset();
  mocks.error.mockReset();
  onClose.mockReset();
  container = document.createElement('div');
  document.body.append(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

describe('sharing an assistant', () => {
  it('shares with the default settings', async () => {
    await render();
    await flush();
    expect(document.body.textContent).toContain('nursery.share.dialog.description');
    expect(document.body.textContent).toContain('nursery.share.dialog.trustNormalHint');
    await click(submit());
    expect(mocks.share).toHaveBeenCalledWith(
      'workspace',
      'Writer',
      { trust: 'normal', sessionRetentionDays: 7, maxConcurrentTasks: 2 },
      { fullTrustRisk: false, externalAgentUsage: false },
    );
    expect(mocks.success).toHaveBeenCalledWith('nursery.share.dialog.shared');
    expect(onClose).toHaveBeenCalled();
  });

  it('asks the owner to accept the risk of full trust', async () => {
    await render();
    await click(byText('nursery.share.dialog.trustFull'));
    expect(document.body.textContent).toContain('nursery.share.dialog.trustFullHint');
    expect(submit().disabled).toBe(true);
    await click(byText('nursery.share.dialog.fullRisk'));
    expect(submit().disabled).toBe(false);
    await click(submit());
    expect(mocks.share).toHaveBeenCalledWith(
      'workspace',
      'Writer',
      expect.objectContaining({ trust: 'full' }),
      { fullTrustRisk: true, externalAgentUsage: false },
    );
  });

  it('notes once that external agents use this device and its plan', async () => {
    mocks.clients.mockResolvedValue([{ id: 'claude-code', enabled: true }]);
    await render();
    await flush();
    expect(document.body.textContent).toContain('nursery.share.dialog.externalAgents');
    expect(submit().disabled).toBe(true);
    await click(byText('nursery.share.dialog.externalAgentsConfirm'));
    await click(submit());
    expect(mocks.share).toHaveBeenCalledWith(
      'workspace',
      'Writer',
      expect.anything(),
      { fullTrustRisk: false, externalAgentUsage: true },
    );
  });

  it('changes the settings of a share without asking again', async () => {
    mocks.clients.mockResolvedValue([{ id: 'claude-code', enabled: true }]);
    await render(existing);
    await flush();
    expect(document.body.textContent).not.toContain('nursery.share.dialog.externalAgents');
    expect(document.body.textContent).not.toContain('nursery.share.dialog.description');
    await click(submit());
    expect(mocks.update).toHaveBeenCalledWith('workspace', {
      name: 'Writer',
      settings: { trust: 'normal', sessionRetentionDays: 30, maxConcurrentTasks: 4 },
      confirmations: { fullTrustRisk: false, externalAgentUsage: false },
    });
    expect(mocks.share).not.toHaveBeenCalled();
  });

  it('keeps the dialog open and says why when sharing fails', async () => {
    mocks.share.mockRejectedValue(new Error('assistant is already shared'));
    await render();
    await click(submit());
    expect(mocks.error).toHaveBeenCalledWith('nursery.share.dialog.failed');
    expect(onClose).not.toHaveBeenCalled();
  });
});
