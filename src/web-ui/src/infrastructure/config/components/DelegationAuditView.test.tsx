// @vitest-environment jsdom
import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DelegationAuditView } from './DelegationAuditView';

const mocks = vi.hoisted(() => ({
  audit: vi.fn(),
  tauri: vi.fn(),
  peer: vi.fn(),
}));
import runtimeCopy from '@/locales/en-US/settings/runtime.json';

/** Echoes the key, and fails on a key or placeholder the copy does not have. */
const translate = (key: string, values?: Record<string, string>) => {
  if (key.startsWith('shared:')) return key;
  const copy = key.split('.').reduce<unknown>(
    (node, part) => (node as Record<string, unknown> | undefined)?.[part], runtimeCopy);
  if (typeof copy !== 'string') throw new Error(`missing copy ${key}`);
  for (const [, name] of copy.matchAll(/{{(\w+)}}/g)) {
    if (values?.[name] === undefined) throw new Error(`${key} needs {{${name}}}`);
  }
  return values ? `${key}:${Object.values(values).join('|')}` : key;
};
vi.mock('@/infrastructure/i18n/hooks/useI18n', () => ({
  useI18n: () => ({ t: translate, formatDate: (value: number) => `date(${value})` }),
}));
vi.mock('@/infrastructure/api/service-api/DelegationAPI', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/infrastructure/api/service-api/DelegationAPI')>()),
  delegationAPI: { listAudit: mocks.audit },
}));
vi.mock('@/infrastructure/runtime', () => ({ isTauriRuntime: mocks.tauri }));
vi.mock('@/infrastructure/peer-device/peerDeviceContextState', () => ({ usePeerDeviceModeOptional: mocks.peer }));
vi.mock('@/shared/utils/logger', () => ({ createLogger: () => ({ error: vi.fn() }) }));
vi.mock('./common', () => ({
  ConfigPageSection: ({ title, children }: { title: string; children: React.ReactNode }) => <section><h2>{title}</h2>{children}</section>,
  ConfigPageRow: ({ label, description, children }: { label: string; description?: string; children: React.ReactNode }) =>
    <div data-row={label}><span>{label}</span><small>{description}</small>{children}</div>,
  ConfigLoadingState: ({ label }: { label: string }) => <p>{label}</p>,
  ConfigRetryState: ({ message }: { message: string }) => <p>{message}</p>,
}));
vi.mock('@openbitfun/ui', () => ({
  Button: ({ variant: _variant, size: _size, ...props }: React.ButtonHTMLAttributes<HTMLButtonElement> & { variant?: string; size?: string }) => <button {...props} />,
}));

const AT = '2026-10-30T10:00:00Z';

function entry(event: string, state?: string, detail?: string, extra: Record<string, unknown> = {}) {
  return {
    entry: {
      at: AT, event, jobId: 'delegation-1', connectionId: 'conn-1', peerUserId: '2002',
      peerDisplayName: 'Bea', peerDeviceId: 'device-b', peerDeviceName: 'B1 Desktop',
      openAgentId: 'helper', taskKind: 'message', state, detail, ...extra,
    },
  };
}

describe('DelegationAuditView', () => {
  let container: HTMLDivElement;
  let root: Root;
  beforeEach(() => {
    Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
    Object.values(mocks).forEach(mock => mock.mockReset());
    mocks.tauri.mockReturnValue(true);
    mocks.peer.mockReturnValue({ peerMode: { active: false } });
    container = document.createElement('div');
    document.body.append(container);
    root = createRoot(container);
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); });
  const render = async () => { await act(async () => root.render(<DelegationAuditView />)); };
  const labels = () => Array.from(container.querySelectorAll('[data-row]')).map(node => node.getAttribute('data-row'));

  it('shows that nothing was delegated yet', async () => {
    mocks.audit.mockResolvedValue([]);
    await render();
    expect(container.textContent).toContain('delegationAudit.empty');
  });

  it('lists tasks newest first with who sent them, and keeps unreadable lines', async () => {
    mocks.audit.mockResolvedValue([
      entry('finished', 'succeeded'),
      entry('submitted'),
      { problem: 'unreadable audit entry: expected value', raw: '{not json' },
      entry('finished', 'failed', 'The task ran longer than the 5-minute limit'),
      entry('paused'),
    ]);
    await render();
    expect(mocks.audit).toHaveBeenCalledWith(100);
    expect(labels()).toEqual([
      'delegationAudit.otherEvent',
      'delegationAudit.failed',
      'delegationAudit.unreadable',
      'delegationAudit.submitted',
      'shared:statuses.done',
      '',
    ]);
    const time = `date(${Date.parse(AT)})`;
    expect(container.textContent).toContain(
      // An entry an earlier build wrote names no trust level.
      `delegationAudit.entry:${time}|helper|Bea|2002|B1 Desktop · The task ran longer than the 5-minute limit`);
    expect(container.textContent).toContain('unreadable audit entry: expected value');
  });

  it('shows the trust level and the external agents a task used', async () => {
    mocks.audit.mockResolvedValue([
      entry('finished', 'succeeded', undefined, { trust: 'full', externalAgents: ['acp:claude-code'] }),
      entry('submitted', undefined, undefined, { trust: 'normal' }),
    ]);
    await render();
    const time = `date(${Date.parse(AT)})`;
    expect(container.textContent).toContain(
      `delegationAudit.entry:${time}|helper|Bea|2002|B1 Desktop · delegationAudit.trustFull · delegationAudit.externalAgents:acp:claude-code`);
    expect(container.textContent).toContain(
      `delegationAudit.entry:${time}|helper|Bea|2002|B1 Desktop · delegationAudit.trustNormal`);
  });

  it('is shown only on the device that keeps the log', async () => {
    mocks.peer.mockReturnValue({ peerMode: { active: true } });
    await render();
    expect(container.textContent).toContain('delegationAudit.unsupported');
    mocks.peer.mockReturnValue({ peerMode: { active: false } });
    mocks.tauri.mockReturnValue(false);
    await render();
    expect(container.textContent).toContain('delegationAudit.unsupported');
    expect(mocks.audit).not.toHaveBeenCalled();
  });

  it('explains a log that cannot be read', async () => {
    mocks.audit.mockRejectedValue(new Error('permission denied'));
    await render();
    expect(container.textContent).toContain('delegationAudit.loadFailed');
  });
});
