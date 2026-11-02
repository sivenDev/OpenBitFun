// @vitest-environment jsdom
import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AgentDelegationConfig } from './AgentDelegationConfig';
import { DELEGATED_TASKS_REFRESH_MS } from '@/infrastructure/api/service-api/delegatedTaskReads';

const mocks = vi.hoisted(() => ({
  list: vi.fn(),
  open: vi.fn(),
  close: vi.fn(),
  grant: vi.fn(),
  revoke: vi.fn(),
  limits: vi.fn(),
  overview: vi.fn(),
  assistants: vi.fn(),
  tasks: vi.fn(),
  devices: vi.fn(),
  deviceInfo: vi.fn(),
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
vi.mock('@/infrastructure/account-identity/accountChanges', () => ({ onSignedInAccountChanged: () => () => undefined }));
vi.mock('@/infrastructure/i18n/hooks/useI18n', () => ({ useI18n: () => ({ t: translate }) }));
vi.mock('@/infrastructure/api/service-api/DelegationAPI', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/infrastructure/api/service-api/DelegationAPI')>()),
  delegationAPI: {
    listOpenAgents: mocks.list, openAgent: mocks.open, closeAgent: mocks.close,
    grantConnection: mocks.grant, revokeConnection: mocks.revoke, setConnectionLimits: mocks.limits,
    listAssistants: mocks.assistants, listTasks: mocks.tasks,
  },
}));
vi.mock('@/infrastructure/api/service-api/ConnectionsAPI', () => ({ connectionsAPI: { overview: mocks.overview } }));
vi.mock('@/infrastructure/contexts/WorkspaceContext', () => ({
  useOptionalWorkspaceContext: () => ({
    assistantWorkspacesList: [{ assistantId: null, name: 'Nova' }],
  }),
  getWorkspaceDisplayName: (workspace: { name: string }) => workspace.name,
}));
vi.mock('@/infrastructure/api/service-api/RemoteConnectAPI', () => ({
  remoteConnectAPI: { accountListDevices: mocks.devices, getDeviceInfo: mocks.deviceInfo },
}));
vi.mock('@/infrastructure/runtime', () => ({ isTauriRuntime: mocks.tauri }));
vi.mock('@/infrastructure/peer-device/peerDeviceContextState', () => ({ usePeerDeviceModeOptional: mocks.peer }));
vi.mock('@/shared/notification-system', () => ({ notificationService: { success: vi.fn(), error: vi.fn() } }));
vi.mock('@/shared/utils/logger', () => ({ createLogger: () => ({ error: vi.fn(), warn: vi.fn() }) }));
vi.mock('./common', () => ({
  ConfigPageSection: ({ title, description, extra, children }: {
    title: string; description?: string; extra?: React.ReactNode; children: React.ReactNode;
  }) => <section data-section={title}><h2>{title}</h2><small>{description}</small>{extra}{children}</section>,
  ConfigPageRow: ({ label, description, children }: { label: string; description?: string; children: React.ReactNode }) =>
    <div data-row={label}><span>{label}</span><small>{description}</small>{children}</div>,
  ConfigLoadingState: ({ label }: { label: string }) => <p>{label}</p>,
  ConfigRetryState: ({ message }: { message: string }) => <p>{message}</p>,
}));
// Form behavior is tested separately from the design system's control rendering.
vi.mock('@openbitfun/ui', () => ({
  Button: ({ variant: _variant, size: _size, ...props }: React.ButtonHTMLAttributes<HTMLButtonElement> & { variant?: string; size?: string }) => <button {...props} />,
  Input: ({ size: _size, ...props }: React.InputHTMLAttributes<HTMLInputElement> & { size?: string }) => <input {...props} />,
  Checkbox: ({ label, ...props }: React.InputHTMLAttributes<HTMLInputElement> & { label: string }) =>
    <label><input type="checkbox" {...props} />{label}</label>,
  Select: ({ value, options, onValueChange, 'aria-label': label }: { value: string; options: { value: string; label: string }[]; onValueChange: (value: string) => void; 'aria-label'?: string }) =>
    <select aria-label={label} value={value} onChange={(event) => onValueChange(event.target.value)}>{options.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}</select>,
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
    mocks.assistants.mockResolvedValue([
      { agentType: 'assistant:workspace', assistantId: null, rootPath: '/home/me/personal_assistant/workspace' },
      { agentType: 'assistant:workspace-work', assistantId: 'work', rootPath: '/home/me/personal_assistant/workspace-work' },
    ]);
    mocks.tasks.mockResolvedValue([]);
    mocks.devices.mockResolvedValue([
      { device_id: 'this-device', device_name: 'This Mac', online: true, last_seen_at: null },
      { device_id: 'laptop', device_name: 'Laptop', online: true, last_seen_at: null },
    ]);
    mocks.deviceInfo.mockResolvedValue({ device_id: 'this-device' });
    mocks.overview.mockRejectedValue(new Error('not signed in'));
    container = document.createElement('div');
    document.body.append(container);
    root = createRoot(container);
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); });
  const render = async () => { await act(async () => root.render(<AgentDelegationConfig />)); };
  const buttons = (label: string) => Array.from(container.querySelectorAll('button')).filter(node => node.textContent === label);
  const button = (label: string) => buttons(label)[0];
  const click = async (label: string) => { await act(async () => button(label)!.click()); };
  const typeInto = async (label: string, value: string) => {
    const input = container.querySelector(`input[aria-label="${label}"]`) as HTMLInputElement;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, value);
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });
  };
  const check = async (label: string) => {
    const input = Array.from(container.querySelectorAll('label')).find(node => node.textContent === label)!.querySelector('input')!;
    await act(async () => input.click());
  };
  const opened = (connections: string[] = [], agentType = 'assistant:workspace') => ({
    id: 'workspace', agentType, displayName: 'Nova', sameAccountDevices: ['laptop'],
    connections: connections.map(connectionId => ({ connectionId })),
  });
  const connection = (connectionId: string, overrides: Record<string, unknown> = {}) => ({
    connectionId, role: 'inviter', status: 'active', usable: true, awaitingConfirmation: false,
    peerUserId: `user-${connectionId}`, peerDisplayName: `Peer ${connectionId}`, peerDeviceId: `device-${connectionId}`,
    peerDeviceName: `Desk ${connectionId}`, createdAt: 1, ...overrides,
  });

  it('lists only the assistants on this device, none of them open by default', async () => {
    await render();
    // Each assistant has its own card.
    expect(container.querySelector('[data-section="Nova"]')!.textContent).toContain('agentDelegation.notOpened');
    // An assistant the workspace list does not name is shown by its id.
    expect(container.querySelector('[data-section="work"]')).not.toBeNull();
    expect(container.textContent).not.toContain('agentDelegation.mode');
    expect(mocks.open).not.toHaveBeenCalled();
  });

  it('opens an assistant to devices of other accounts, each on its own, after the owner confirms external agent use', async () => {
    mocks.overview.mockResolvedValue({
      connections: [connection('conn-1'), connection('conn-2'), connection('conn-3', { status: 'pending', usable: false })],
    });
    mocks.open.mockResolvedValue({});
    mocks.grant.mockResolvedValue({});
    await render();
    await click('agentDelegation.open');
    // Opening goes straight to the devices of other accounts; this account's devices are not offered.
    expect(container.textContent).not.toContain('Laptop');
    expect(container.textContent).not.toContain('agentDelegation.allDevices');
    const offered = Array.from(container.querySelectorAll('label')).map(node => node.textContent)
      .filter(text => text?.startsWith('agentDelegation.connectionLabel'));
    expect(offered).toEqual([
      'agentDelegation.connectionLabel:Peer conn-1|user-conn-1|Desk conn-1',
      'agentDelegation.connectionLabel:Peer conn-2|user-conn-2|Desk conn-2',
    ]);
    await check('agentDelegation.connectionLabel:Peer conn-1|user-conn-1|Desk conn-1');
    await check('agentDelegation.connectionLabel:Peer conn-2|user-conn-2|Desk conn-2');
    expect(button('agentDelegation.save')!.disabled).toBe(true);
    await check('agentDelegation.usageConfirm');
    expect(button('agentDelegation.save')!.disabled).toBe(false);
    await click('agentDelegation.save');
    expect(mocks.open).toHaveBeenCalledWith({
      id: 'workspace',
      agentType: 'assistant:workspace',
      displayName: 'Nova',
      description: null,
      sameAccountDevices: [],
      usageConfirmed: true,
    });
    expect(mocks.grant.mock.calls).toEqual([['workspace', 'conn-1'], ['workspace', 'conn-2']]);
  });

  it('cannot open an assistant before a device of another account is paired', async () => {
    await render();
    await click('agentDelegation.open');
    expect(container.textContent).toContain('agentDelegation.noConnections');
    expect(button('agentDelegation.save')).toBeUndefined();
  });

  it('names the devices of this account an assistant is opened to', async () => {
    mocks.list.mockResolvedValue([opened()]);
    await render();
    expect(container.querySelector('[data-section="Nova"]')!.textContent).toContain('agentDelegation.openedTo:Laptop');
  });

  it('grants an opened assistant to more devices, each on its own, and withdraws it', async () => {
    mocks.overview.mockResolvedValue({
      connections: [
        connection('conn-1'),
        connection('conn-2', { status: 'pending', usable: false }),
        connection('conn-3', { status: 'revoked', usable: false }),
      ],
    });
    mocks.list
      .mockResolvedValueOnce([opened()])
      .mockResolvedValue([opened(['conn-1', 'conn-gone'])]);
    mocks.grant.mockResolvedValue({});
    await render();
    await click('agentDelegation.grantConnection');
    // Only usable connections are offered, and nothing is chosen for the owner.
    const offered = Array.from(container.querySelectorAll('label')).map(node => node.textContent)
      .filter(text => text?.startsWith('agentDelegation.connectionLabel'));
    expect(offered).toEqual(['agentDelegation.connectionLabel:Peer conn-1|user-conn-1|Desk conn-1']);
    expect(button('agentDelegation.grant')!.disabled).toBe(true);
    await check('agentDelegation.connectionLabel:Peer conn-1|user-conn-1|Desk conn-1');
    await click('agentDelegation.grant');
    expect(mocks.grant).toHaveBeenCalledWith('workspace', 'conn-1');

    // Each grant is named by whom it goes to.
    expect(container.querySelector('[data-row="agentDelegation.connectionLabel:Peer conn-1|user-conn-1|Desk conn-1"]')!.textContent)
      .toContain('agentDelegation.grantedWith:agentDelegation.limits.anyJobs');
    // Sessions are kept 7 days unless the owner chooses another period.
    expect(container.querySelector('[data-row="agentDelegation.connectionLabel:Peer conn-1|user-conn-1|Desk conn-1"]')!.textContent)
      .toContain('agentDelegation.limits.retention:7');
    expect(container.querySelector('[data-row="agentDelegation.connectionMissing:conn-gone"]')).not.toBeNull();
    // Limits are set per device, after it is granted.
    mocks.limits.mockResolvedValue({});
    await click('agentDelegation.limits.edit');
    await typeInto('agentDelegation.limits.jobsLabel', '0');
    expect(button('agentDelegation.limits.save')!.disabled).toBe(true);
    await typeInto('agentDelegation.limits.jobsLabel', '2');
    const retention = container.querySelector('select[aria-label="agentDelegation.limits.retentionLabel"]') as HTMLSelectElement;
    expect(Array.from(retention.options).map(option => option.value)).toEqual(['1', '7', '30', '90']);
    expect(retention.value).toBe('7');
    await act(async () => {
      retention.value = '30';
      retention.dispatchEvent(new Event('change', { bubbles: true }));
    });
    await click('agentDelegation.limits.save');
    expect(mocks.limits).toHaveBeenCalledWith('workspace', 'conn-1',
      { maxConcurrentJobs: 2, maxJobMinutes: undefined, sessionRetentionDays: 30 });
    mocks.revoke.mockResolvedValue(true);
    await click('agentDelegation.withdraw');
    expect(mocks.revoke).toHaveBeenCalledWith('workspace', 'conn-1');
  });

  it('counts outside tasks on their assistant and leaves the sessions to the assistant', async () => {
    mocks.list.mockResolvedValue([opened()]);
    const task = (jobId: string, agentType: string, pendingPermissionCount = 0) => ({
      jobId, sessionId: jobId, state: 'running', title: `Task ${jobId}`, openAgentId: 'workspace', agentType,
      pendingPermissionCount, source: { kind: 'connection', connectionId: 'conn-1', accountName: 'Bea', deviceName: 'B1' },
    });
    mocks.tasks.mockResolvedValue([
      task('a', 'assistant:workspace', 2), task('b', 'assistant:workspace'), task('c', 'assistant:workspace-work'),
    ]);
    await render();
    expect(container.querySelector('[data-section="Nova"] small')!.textContent)
      .toBe('agentDelegation.openedTo:Laptop · agentDelegation.taskCount:2 · agentDelegation.tasksWaiting:1');
    expect(container.querySelector('[data-section="work"] small')!.textContent)
      .toBe('agentDelegation.notOpened · agentDelegation.taskCount:1');
    // No task rows: each task is read and approved as its session under the assistant.
    expect(container.textContent).not.toContain('Task a');
  });

  it('asks before it stops opening an assistant', async () => {
    mocks.list.mockResolvedValue([opened()]);
    mocks.close.mockResolvedValue(true);
    await render();
    await click('agentDelegation.close');
    expect(mocks.close).not.toHaveBeenCalled();
    expect(container.textContent).toContain('agentDelegation.closeConfirmDescription');
    await act(async () => container.querySelector<HTMLButtonElement>(
      '[data-row="agentDelegation.closeConfirmTitle:Nova"] button:last-child')!.click());
    expect(mocks.close).toHaveBeenCalledWith('workspace');
  });

  it('reads tasks again while the page stays open', async () => {
    vi.useFakeTimers();
    try {
      await render();
      expect(container.textContent).not.toContain('agentDelegation.taskCount');
      mocks.tasks.mockResolvedValue([{
        jobId: 'delegation-1', sessionId: 's1', state: 'running', title: 'Plan the week', openAgentId: 'workspace',
        agentType: 'assistant:workspace', assistantWorkspace: '/home/me/personal_assistant/workspace',
        pendingPermissionCount: 1, source: { kind: 'device', deviceId: 'laptop', deviceName: 'Laptop' },
      }]);
      await act(async () => { await vi.advanceTimersByTimeAsync(DELEGATED_TASKS_REFRESH_MS); });
      expect(container.textContent).toContain('agentDelegation.taskCount:1');
    } finally {
      vi.useRealTimers();
    }
  });

  it('still manages assistants when the task list cannot be read', async () => {
    mocks.tasks.mockRejectedValue(new Error('dispatch runner is not installed'));
    await render();
    expect(container.querySelector('[data-section="Nova"]')).not.toBeNull();
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
