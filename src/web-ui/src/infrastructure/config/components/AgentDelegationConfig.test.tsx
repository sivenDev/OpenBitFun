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
  status: vi.fn(),
  answer: vi.fn(),
  audit: vi.fn(),
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
// Action labels come from the chat namespace; they are echoed as keys.
vi.mock('@/infrastructure/i18n/hooks/useI18n', () => ({
  useI18n: (namespace: string) => ({ t: namespace === 'flow-chat' ? (key: string) => key : translate }),
}));
vi.mock('@/infrastructure/api/service-api/DelegationAPI', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/infrastructure/api/service-api/DelegationAPI')>()),
  delegationAPI: {
    listOpenAgents: mocks.list, openAgent: mocks.open, closeAgent: mocks.close,
    grantConnection: mocks.grant, revokeConnection: mocks.revoke, setConnectionLimits: mocks.limits,
    listAssistants: mocks.assistants, listTasks: mocks.tasks, taskStatus: mocks.status, answerTask: mocks.answer,
    listAudit: mocks.audit,
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
    mocks.assistants.mockResolvedValue([
      { agentType: 'assistant:workspace', assistantId: null, rootPath: '/home/me/personal_assistant/workspace' },
      { agentType: 'assistant:workspace-work', assistantId: 'work', rootPath: '/home/me/personal_assistant/workspace-work' },
    ]);
    mocks.tasks.mockResolvedValue([]);
    mocks.status.mockResolvedValue({ state: 'running', cursor: 0, pendingPermissions: [] });
    mocks.audit.mockResolvedValue([]);
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
    agent: {
      id: 'workspace', agentType, displayName: 'Nova', taskKinds: ['message'], sameAccountDevices: ['laptop'],
      connections: connections.map(connectionId => ({ connectionId })),
    },
  });
  const connection = (connectionId: string, overrides: Record<string, unknown> = {}) => ({
    connectionId, role: 'inviter', status: 'active', usable: true, awaitingConfirmation: false,
    peerUserId: `user-${connectionId}`, peerDisplayName: `Peer ${connectionId}`, peerDeviceId: `device-${connectionId}`,
    peerDeviceName: `Desk ${connectionId}`, createdAt: 1, ...overrides,
  });

  it('lists only the assistants on this device, none of them open by default', async () => {
    await render();
    expect(container.querySelector('[data-row="Nova"]')!.textContent).toContain('agentDelegation.notOpened');
    // An assistant the workspace list does not name is shown by its id.
    expect(container.querySelector('[data-row="work"]')).not.toBeNull();
    expect(container.textContent).not.toContain('agentDelegation.mode');
    expect(mocks.open).not.toHaveBeenCalled();
  });

  it('opens an assistant only after the owner confirms external agent use', async () => {
    mocks.open.mockResolvedValue({});
    await render();
    await click('agentDelegation.open');
    // The current device is never offered as a delegation source.
    expect(container.textContent).not.toContain('This Mac');
    await check('Laptop');
    expect(button('agentDelegation.save')!.disabled).toBe(true);
    await check('agentDelegation.usageConfirm');
    expect(button('agentDelegation.save')!.disabled).toBe(false);
    await click('agentDelegation.save');
    expect(mocks.open).toHaveBeenCalledWith({
      id: 'workspace',
      agentType: 'assistant:workspace',
      displayName: 'Nova',
      description: null,
      sameAccountDevices: ['laptop'],
      usageConfirmed: true,
    });
  });

  it('marks entries of earlier builds as no longer supported and deletes them', async () => {
    mocks.list.mockResolvedValue([
      { problem: 'unreadable entry: unknown variant', raw: { id: 'future' } },
      { agent: { id: 'coder', agentType: 'acp:claude-code', displayName: 'Coder', taskKinds: ['coding'], sameAccountDevices: ['*'] } },
      opened(),
    ]);
    mocks.close.mockResolvedValue(true);
    await render();
    expect(container.querySelector('[data-row="Coder"]')!.textContent).toContain('agentDelegation.unsupportedEntry');
    expect(container.textContent).toContain('agentDelegation.unreadable:unreadable entry: unknown variant');
    expect(container.querySelector('[data-row="Nova"]')!.textContent).toContain('agentDelegation.openedTo:Laptop');
    await act(async () => container.querySelector<HTMLButtonElement>('[data-row="Coder"] button')!.click());
    expect(mocks.close).toHaveBeenCalledWith('coder');
    await act(async () => container.querySelector<HTMLButtonElement>('[data-row="future"] button')!.click());
    expect(mocks.close).toHaveBeenCalledWith('future');
  });

  it('grants an opened assistant to a usable connection, with limits, and withdraws it', async () => {
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
    mocks.limits.mockResolvedValue({});
    await render();
    await click('agentDelegation.grantConnection');
    // Only usable connections are offered, and the first is preselected.
    expect(Array.from(container.querySelectorAll('option')).map(option => option.value)).toEqual(['conn-1']);
    await typeInto('agentDelegation.limits.jobsLabel', '0');
    expect(button('agentDelegation.grant')!.disabled).toBe(true);
    await typeInto('agentDelegation.limits.jobsLabel', '2');
    await click('agentDelegation.grant');
    expect(mocks.grant).toHaveBeenCalledWith('workspace', 'conn-1');
    expect(mocks.limits).toHaveBeenCalledWith('workspace', 'conn-1', { maxConcurrentJobs: 2, maxJobMinutes: undefined });

    // Each grant is named by whom it goes to.
    expect(container.querySelector('[data-row="agentDelegation.connectionLabel:Peer conn-1|user-conn-1|Desk conn-1"]')!.textContent)
      .toContain('agentDelegation.limits.anyJobs');
    expect(container.querySelector('[data-row="agentDelegation.connectionMissing:conn-gone"]')).not.toBeNull();
    mocks.revoke.mockResolvedValue(true);
    await click('agentDelegation.withdraw');
    expect(mocks.revoke).toHaveBeenCalledWith('workspace', 'conn-1');
  });

  it('lists outside tasks under their assistant with their source, and approves one action at a time', async () => {
    mocks.list.mockResolvedValue([opened()]);
    mocks.tasks.mockResolvedValue([
      {
        jobId: 'delegation-1', sessionId: 's1', state: 'running', title: 'Plan the trip',
        openAgentId: 'workspace', agentType: 'assistant:workspace', pendingPermissionCount: 3,
        source: { kind: 'connection', connectionId: 'conn-1', accountName: 'Bea', deviceName: 'B1' },
      },
      {
        jobId: 'delegation-2', sessionId: 's2', state: 'succeeded', title: 'Summarize notes',
        openAgentId: 'work', agentType: 'assistant:workspace-work', pendingPermissionCount: 0,
        source: { kind: 'device', deviceId: 'laptop', deviceName: 'Laptop' },
      },
    ]);
    mocks.status.mockResolvedValue({
      state: 'running', cursor: 3,
      pendingPermissions: [
        { requestId: 'req-1', action: 'bash', resources: ['ls -la'] },
        // Paths inside the assistant's folder are shown relative to it, however the runner spells the folder.
        { requestId: 'req-2', action: 'read', resources: ['/private/home/me/personal_assistant/workspace/USER.md'] },
        { requestId: 'req-3', action: 'external_directory', resources: ['/home/me/personal_assistant/workspace/'] },
      ],
    });
    mocks.answer.mockResolvedValue(undefined);
    await render();
    const rows = Array.from(container.querySelectorAll('[data-row]')).map(node => node.getAttribute('data-row'));
    // Each task follows the assistant that runs it.
    expect(rows.indexOf('Plan the trip')).toBe(rows.indexOf('Nova') + 1);
    expect(rows.indexOf('Summarize notes')).toBe(rows.indexOf('work') + 1);
    const trip = container.querySelector('[data-row="Plan the trip"]')!.textContent!;
    expect(trip).toContain('agentDelegation.tasks.fromAccount:Bea|B1');
    expect(trip).toContain('agentDelegation.tasks.waiting:3');
    expect(container.querySelector('[data-row="Summarize notes"]')!.textContent)
      .toContain('agentDelegation.tasks.fromDevice:Laptop');

    // What a waiting task asks for is shown without opening it.
    expect(mocks.status).toHaveBeenCalledWith('delegation-1');
    expect(mocks.status).not.toHaveBeenCalledWith('delegation-2');
    expect(rows).not.toContain('permission.actions.other');
    expect(container.querySelector('[data-row="permission.actions.bash"]')!.textContent).toContain('ls -la');
    expect(container.querySelector('[data-row="permission.actions.read"] small')!.textContent).toBe('USER.md');
    expect(container.querySelector('[data-row="permission.actions.externalDirectory"] small')!.textContent)
      .toBe('agentDelegation.tasks.assistantFolder');
    // Only one-off answers: the owner approves each action of another device's task.
    expect(button('agentDelegation.tasks.allowOnce')).toBeDefined();
    await click('agentDelegation.tasks.allowOnce');
    expect(mocks.answer).toHaveBeenCalledWith('delegation-1', 'req-1', 'once');
    // The runner lists an answered prompt until it consumes the answer; it is not offered again.
    expect(container.querySelector('[data-row="permission.actions.bash"]')).toBeNull();
    expect(container.querySelector('[data-row="Plan the trip"]')!.textContent).toContain('agentDelegation.tasks.waiting:2');
  });

  it('lists running tasks and only the last finished ones', async () => {
    mocks.tasks.mockResolvedValue([
      { jobId: 'run', state: 'running', title: 'Running', agentType: 'assistant:workspace', pendingPermissionCount: 0,
        source: { kind: 'device', deviceId: 'laptop' } },
      ...['a', 'b', 'c', 'd'].map(id => ({ jobId: id, state: 'succeeded', title: `Done ${id}`,
        agentType: 'assistant:workspace', pendingPermissionCount: 0, source: { kind: 'device', deviceId: 'laptop' } })),
    ]);
    await render();
    const shown = () => Array.from(container.querySelectorAll('[data-row]')).map(node => node.getAttribute('data-row'))
      .filter(row => row?.startsWith('Done') || row === 'Running');
    expect(shown()).toEqual(['Running', 'Done a', 'Done b', 'Done c']);
    await click('agentDelegation.tasks.showEarlier:1');
    expect(shown()).toEqual(['Running', 'Done a', 'Done b', 'Done c', 'Done d']);
  });

  it('shows what the record kept about a task from another account', async () => {
    const at = '2026-10-30T10:00:00Z';
    const recorded = (event: string, extra: Record<string, unknown>) => ({ entry: {
      at, event, jobId: 'job-1', connectionId: 'conn-1', peerUserId: '2002', peerDisplayName: 'Bea',
      peerDeviceId: 'b1', peerDeviceName: 'B1', openAgentId: 'workspace', taskKind: 'message', ...extra,
    } });
    mocks.audit.mockResolvedValue([
      recorded('submitted', { trust: 'full' }),
      { problem: 'unreadable line' },
      recorded('finished', { state: 'failed', externalAgents: ['acp:claude-code'], detail: 'Ran past 5 minutes' }),
    ]);
    mocks.tasks.mockResolvedValue([{
      jobId: 'job-1', state: 'failed', title: 'Draft the plan', agentType: 'assistant:workspace',
      pendingPermissionCount: 0, source: { kind: 'connection', connectionId: 'conn-1', accountName: 'Bea', deviceName: 'B1' },
    }]);
    await render();
    expect(mocks.audit).toHaveBeenCalledWith(500);
    expect(container.querySelector('[data-row="Draft the plan"]')!.textContent).toContain(
      'agentDelegation.tasks.fromAccount:Bea|B1 · shared:statuses.failed · agentDelegation.tasks.trustFull'
      + ' · agentDelegation.tasks.externalAgents:acp:claude-code · Ran past 5 minutes');
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
      expect(container.textContent).not.toContain('Plan the week');
      mocks.tasks.mockResolvedValue([{
        jobId: 'delegation-1', sessionId: 's1', state: 'running', title: 'Plan the week', openAgentId: 'workspace',
        agentType: 'assistant:workspace', assistantWorkspace: '/home/me/personal_assistant/workspace',
        pendingPermissionCount: 1, source: { kind: 'device', deviceId: 'laptop', deviceName: 'Laptop' },
      }]);
      await act(async () => { await vi.advanceTimersByTimeAsync(DELEGATED_TASKS_REFRESH_MS); });
      expect(container.textContent).toContain('Plan the week');
    } finally {
      vi.useRealTimers();
    }
  });

  it('still manages assistants when the task list cannot be read', async () => {
    mocks.tasks.mockRejectedValue(new Error('dispatch runner is not installed'));
    await render();
    expect(container.querySelector('[data-row="Nova"]')).not.toBeNull();
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
