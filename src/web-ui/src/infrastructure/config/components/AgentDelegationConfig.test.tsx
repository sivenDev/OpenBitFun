// @vitest-environment jsdom
import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AgentDelegationConfig } from './AgentDelegationConfig';

const mocks = vi.hoisted(() => ({
  list: vi.fn(),
  open: vi.fn(),
  close: vi.fn(),
  modes: vi.fn(),
  clients: vi.fn(),
  devices: vi.fn(),
  deviceInfo: vi.fn(),
  tauri: vi.fn(),
  peer: vi.fn(),
}));
import runtimeCopy from '@/locales/en-US/settings/runtime.json';

/** Echoes the key, and fails on a key or placeholder the copy does not have. */
const translate = (key: string, values?: Record<string, string>) => {
  const copy = key.split('.').reduce<unknown>(
    (node, part) => (node as Record<string, unknown> | undefined)?.[part], runtimeCopy);
  if (typeof copy !== 'string') throw new Error(`missing copy ${key}`);
  for (const [, name] of copy.matchAll(/{{(\w+)}}/g)) {
    if (values?.[name] === undefined) throw new Error(`${key} needs {{${name}}}`);
  }
  return values ? `${key}:${Object.values(values).join('|')}` : key;
};
vi.mock('@/infrastructure/i18n/hooks/useI18n', () => ({ useI18n: () => ({ t: translate }) }));
vi.mock('@/infrastructure/api/service-api/DelegationAPI', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/infrastructure/api/service-api/DelegationAPI')>()),
  delegationAPI: { listOpenAgents: mocks.list, openAgent: mocks.open, closeAgent: mocks.close },
}));
vi.mock('@/infrastructure/api/service-api/AgentAPI', () => ({ agentAPI: { getAvailableModes: mocks.modes } }));
vi.mock('@/infrastructure/api/service-api/ACPClientAPI', () => ({ ACPClientAPI: { getClients: mocks.clients } }));
vi.mock('@/infrastructure/api/service-api/RemoteConnectAPI', () => ({
  remoteConnectAPI: { accountListDevices: mocks.devices, getDeviceInfo: mocks.deviceInfo },
}));
vi.mock('@/infrastructure/runtime', () => ({ isTauriRuntime: mocks.tauri }));
vi.mock('@/infrastructure/peer-device/peerDeviceContextState', () => ({ usePeerDeviceModeOptional: mocks.peer }));
vi.mock('@/shared/notification-system', () => ({ notificationService: { success: vi.fn(), error: vi.fn() } }));
vi.mock('@/shared/utils/logger', () => ({ createLogger: () => ({ error: vi.fn() }) }));
vi.mock('./common', () => ({
  ConfigPageSection: ({ title, children }: { title: string; children: React.ReactNode }) => <section><h2>{title}</h2>{children}</section>,
  ConfigPageRow: ({ label, description, children }: { label: string; description?: string; children: React.ReactNode }) =>
    <div data-row={label}><span>{label}</span><small>{description}</small>{children}</div>,
  ConfigLoadingState: ({ label }: { label: string }) => <p>{label}</p>,
  ConfigRetryState: ({ message }: { message: string }) => <p>{message}</p>,
}));
// Form behavior is tested separately from the design system's control rendering.
vi.mock('@openbitfun/ui', () => ({
  Button: ({ variant: _variant, size: _size, ...props }: React.ButtonHTMLAttributes<HTMLButtonElement> & { variant?: string; size?: string }) => <button {...props} />,
  Input: ({ size: _size, ...props }: React.InputHTMLAttributes<HTMLInputElement> & { size?: string }) => <input {...props} />,
  Switch: (props: React.InputHTMLAttributes<HTMLInputElement>) => <input type="checkbox" data-switch {...props} />,
  Checkbox: ({ label, ...props }: React.InputHTMLAttributes<HTMLInputElement> & { label: string }) =>
    <label><input type="checkbox" {...props} />{label}</label>,
  Select: ({ value, options, onValueChange }: { value: string; options: { value: string; label: string }[]; onValueChange: (value: string) => void }) =>
    <select value={value} onChange={(event) => onValueChange(event.target.value)}>{options.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}</select>,
}));

describe('AgentDelegationConfig', () => {
  let container: HTMLDivElement;
  let root: Root;
  beforeEach(() => {
    Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
    Object.values(mocks).forEach(mock => mock.mockReset());
    mocks.tauri.mockReturnValue(true);
    mocks.peer.mockReturnValue({ peerMode: { active: false } });
    mocks.list.mockResolvedValue([]);
    mocks.modes.mockResolvedValue([{ id: 'Standard', name: 'Standard' }]);
    mocks.clients.mockResolvedValue([]);
    mocks.devices.mockResolvedValue([
      { device_id: 'this-device', device_name: 'This Mac', online: true, last_seen_at: null },
      { device_id: 'laptop', device_name: 'Laptop', online: true, last_seen_at: null },
    ]);
    mocks.deviceInfo.mockResolvedValue({ device_id: 'this-device' });
    container = document.createElement('div');
    document.body.append(container);
    root = createRoot(container);
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); });
  const render = async () => { await act(async () => root.render(<AgentDelegationConfig />)); };
  const button = (label: string) => Array.from(container.querySelectorAll('button')).find(node => node.textContent === label);
  const click = async (label: string) => { await act(async () => button(label)!.click()); };
  const check = async (label: string) => {
    const input = Array.from(container.querySelectorAll('label')).find(node => node.textContent === label)!.querySelector('input')!;
    await act(async () => input.click());
  };

  it('shows that nothing is open by default', async () => {
    await render();
    expect(container.textContent).toContain('agentDelegation.empty');
    expect(mocks.open).not.toHaveBeenCalled();
  });

  it('opens an agent only to the chosen device of this account', async () => {
    mocks.open.mockResolvedValue({});
    await render();
    await click('agentDelegation.open');
    // The current device is never offered as a delegation source.
    expect(container.textContent).not.toContain('This Mac');
    expect(button('agentDelegation.save')!.disabled).toBe(true);
    await check('Laptop');
    expect(button('agentDelegation.save')!.disabled).toBe(false);
    await click('agentDelegation.save');
    expect(mocks.open).toHaveBeenCalledWith({
      id: 'standard',
      agentType: 'Standard',
      displayName: 'Standard',
      description: null,
      taskKinds: ['message'],
      messageWorkspace: null,
      sameAccountDevices: ['laptop'],
    });
  });

  it('opens an enabled external agent only after its usage notice is confirmed', async () => {
    mocks.clients.mockResolvedValue([
      { id: 'claude-code', name: 'Claude Code', enabled: true },
      { id: 'codex', name: 'Codex', enabled: true },
      { id: 'gemini', name: 'Gemini', enabled: false },
      { id: 'opencode', name: 'OpenCode', enabled: true, subagent: { enabled: false } },
    ]);
    mocks.open.mockResolvedValue({});
    await render();
    await click('agentDelegation.open');
    const options = Array.from(container.querySelectorAll('option')).map(option => option.value);
    // Disabled clients, and clients that refuse delegated work, are not offered.
    expect(options).toEqual(['Standard', 'acp:claude-code', 'acp:codex']);
    expect(container.textContent).not.toContain('agentDelegation.usageNotice');

    const select = container.querySelector('select')!;
    await act(async () => {
      select.value = 'acp:claude-code';
      select.dispatchEvent(new Event('change', { bubbles: true }));
    });
    expect(container.textContent).toContain('agentDelegation.usageNotice agentDelegation.usageNoticeClaudeCode');
    await check('Laptop');
    expect(button('agentDelegation.save')!.disabled).toBe(true);
    await check('agentDelegation.usageConfirm');
    expect(button('agentDelegation.save')!.disabled).toBe(false);

    // Switching agents asks again, and other agents show only the provider notice.
    await act(async () => {
      select.value = 'acp:codex';
      select.dispatchEvent(new Event('change', { bubbles: true }));
    });
    expect(container.textContent).not.toContain('agentDelegation.usageNoticeClaudeCode');
    expect(button('agentDelegation.save')!.disabled).toBe(true);
    await check('agentDelegation.usageConfirm');
    await click('agentDelegation.save');
    expect(mocks.open).toHaveBeenCalledWith(expect.objectContaining({
      id: 'acp-codex',
      agentType: 'acp:codex',
      displayName: 'Codex',
      sameAccountDevices: ['laptop'],
      usageConfirmed: true,
    }));
  });

  it('opens local modes when the host has no external agents', async () => {
    mocks.clients.mockRejectedValue(new Error('no ACP'));
    await render();
    await click('agentDelegation.open');
    expect(Array.from(container.querySelectorAll('option')).map(option => option.value)).toEqual(['Standard']);
  });

  it('lists unreadable entries as closed and can still close them', async () => {
    mocks.list.mockResolvedValue([
      { problem: 'unreadable entry: unknown variant', raw: { id: 'future' } },
      {
        agent: {
          id: 'helper', agentType: 'Standard', displayName: 'Helper',
          taskKinds: ['message', 'telepathy'], sameAccountDevices: ['laptop'],
        },
      },
    ]);
    mocks.close.mockResolvedValue(true);
    await render();
    expect(container.textContent).toContain('agentDelegation.unreadable:unreadable entry: unknown variant');
    expect(container.textContent).toContain('agentDelegation.summary:Standard|agentDelegation.taskKindMessage|Laptop');
    await act(async () => container.querySelector<HTMLButtonElement>('[data-row="future"] button')!.click());
    expect(mocks.close).toHaveBeenCalledWith('future');
  });

  it('refuses to manage the list for a connected peer or without the desktop host', async () => {
    mocks.peer.mockReturnValue({ peerMode: { active: true, deviceId: 'laptop', deviceName: 'Laptop' } });
    await render();
    expect(container.textContent).toContain('agentDelegation.peerUnsupported');
    expect(mocks.list).not.toHaveBeenCalled();

    mocks.peer.mockReturnValue({ peerMode: { active: false } });
    mocks.tauri.mockReturnValue(false);
    await render();
    expect(container.textContent).toContain('agentDelegation.unsupported');
    expect(mocks.list).not.toHaveBeenCalled();
  });
});
