// @vitest-environment jsdom
import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ConnectionsConfig } from './ConnectionsConfig';
import type {
  ConnectionsOverview,
  ConnectionSummary,
} from '@/infrastructure/api/service-api/ConnectionsAPI';

const mocks = vi.hoisted(() => ({
  overview: vi.fn(),
  createInvite: vi.fn(),
  previewInvite: vi.fn(),
  acceptInvite: vi.fn(),
  confirm: vi.fn(),
  revoke: vi.fn(),
  withdrawInvite: vi.fn(),
  listTrust: vi.fn(),
  setTrust: vi.fn(),
  listOpenAgents: vi.fn(),
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
vi.mock('@/infrastructure/i18n/hooks/useI18n', () => ({
  useI18n: () => ({ t: translate, formatDate: (value: number) => `date(${value})` }),
}));
vi.mock('@/infrastructure/api/service-api/ConnectionsAPI', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/infrastructure/api/service-api/ConnectionsAPI')>()),
  connectionsAPI: {
    overview: mocks.overview,
    createInvite: mocks.createInvite,
    previewInvite: mocks.previewInvite,
    acceptInvite: mocks.acceptInvite,
    confirm: mocks.confirm,
    revoke: mocks.revoke,
    withdrawInvite: mocks.withdrawInvite,
  },
}));
vi.mock('@/infrastructure/api/service-api/DelegationAPI', () => ({
  delegationAPI: {
    listConnectionTrust: mocks.listTrust, setConnectionTrust: mocks.setTrust,
    listOpenAgents: mocks.listOpenAgents,
  },
  isAssistantAgentType: (agentType: string) => agentType.startsWith('assistant:'),
}));
vi.mock('@/infrastructure/runtime', () => ({ isTauriRuntime: mocks.tauri }));
vi.mock('@/infrastructure/peer-device/peerDeviceContextState', () => ({ usePeerDeviceModeOptional: mocks.peer }));
vi.mock('@/shared/notification-system', () => ({ notificationService: { success: vi.fn(), error: vi.fn() } }));
vi.mock('@/shared/utils/nativeClipboard', () => ({ writeNativeClipboardText: vi.fn().mockResolvedValue(true) }));
vi.mock('@/shared/utils/logger', () => ({ createLogger: () => ({ error: vi.fn() }) }));
vi.mock('./common', () => ({
  ConfigPageSection: ({ title, description, extra, children }: {
    title: string; description?: string; extra?: React.ReactNode; children: React.ReactNode;
  }) => <section data-section={title}><h2>{title}</h2><small>{description}</small>{extra}{children}</section>,
  ConfigPageRow: ({ label, description, children }: { label: string; description?: string; children: React.ReactNode }) =>
    <div data-row={label}><span>{label}</span><small>{description}</small>{children}</div>,
  ConfigLoadingState: ({ label }: { label: string }) => <p>{label}</p>,
  ConfigRetryState: ({ message }: { message: string }) => <p>{message}</p>,
}));
vi.mock('@openbitfun/ui', () => ({
  Button: ({ variant: _variant, size: _size, ...props }: React.ButtonHTMLAttributes<HTMLButtonElement> & { variant?: string; size?: string }) => <button {...props} />,
  Input: ({ size: _size, ...props }: React.InputHTMLAttributes<HTMLInputElement> & { size?: string }) => <input {...props} />,
  Select: ({ value, options, onValueChange, 'aria-label': label }: {
    value: string; options: { value: string; label: string }[]; onValueChange: (value: string) => void; 'aria-label'?: string;
  }) => (
    <select aria-label={label} value={value} onChange={event => onValueChange(event.target.value)}>
      {options.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}
    </select>
  ),
}));

const FINGERPRINT_A = 'AAAA BBBB CCCC DDDD EEEE FFFF 0000 1111';
const FINGERPRINT_B = '2222 3333 4444 5555 6666 7777 8888 9999';

function connection(overrides: Partial<ConnectionSummary> = {}): ConnectionSummary {
  return {
    connectionId: 'conn-1',
    role: 'inviter',
    status: 'active',
    usable: true,
    unusableReason: null,
    awaitingConfirmation: false,
    peerUserId: '2002',
    peerDisplayName: 'Bea',
    peerDeviceId: 'device-b',
    peerDeviceName: 'B1 Desktop',
    peerFingerprint: FINGERPRINT_B,
    createdAt: 1,
    confirmedAt: 2,
    revokedAt: null,
    ...overrides,
  };
}

function overview(overrides: Partial<ConnectionsOverview> = {}): ConnectionsOverview {
  return {
    supported: true,
    relayError: null,
    localDeviceId: 'device-a',
    localFingerprint: FINGERPRINT_A,
    connections: [],
    unreadable: [],
    invites: [],
    ...overrides,
  };
}

describe('ConnectionsConfig', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
    Object.values(mocks).forEach(mock => mock.mockReset());
    mocks.tauri.mockReturnValue(true);
    mocks.peer.mockReturnValue({ peerMode: { active: false } });
    mocks.overview.mockResolvedValue(overview());
    mocks.listTrust.mockResolvedValue({});
    mocks.listOpenAgents.mockResolvedValue([]);
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  const render = async () => {
    await act(async () => { root.render(<ConnectionsConfig />); });
  };
  const button = (label: string) => {
    const found = [...container.querySelectorAll('button')].find(item => item.textContent === label);
    if (!found) throw new Error(`no button ${label}: ${container.innerHTML}`);
    return found;
  };
  const click = async (label: string) => {
    await act(async () => { button(label).click(); });
  };
  const type = async (value: string) => {
    const input = container.querySelector('input') as HTMLInputElement;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
      setter.call(input, value);
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });
  };

  const chooseTrust = async (level: string) => {
    const select = container.querySelector('select[aria-label="connections.trust.label"]') as HTMLSelectElement;
    await act(async () => {
      select.value = level;
      select.dispatchEvent(new Event('change', { bubbles: true }));
    });
  };

  it('shows an empty list; the fingerprint appears only while pairing', async () => {
    await render();
    expect(container.textContent).toContain('connections.empty');
    expect(container.textContent).not.toContain(FINGERPRINT_A);
  });

  it('is unavailable outside the desktop app and for a connected peer device', async () => {
    mocks.tauri.mockReturnValue(false);
    await render();
    expect(container.textContent).toContain('connections.unsupported');
    mocks.tauri.mockReturnValue(true);
    mocks.peer.mockReturnValue({ peerMode: { active: true } });
    await render();
    expect(container.textContent).toContain('connections.peerUnsupported');
    expect(mocks.overview).not.toHaveBeenCalled();
  });

  it('explains a relay without cross-account routing and disables pairing', async () => {
    mocks.overview.mockResolvedValue(overview({ supported: false }));
    await render();
    expect(container.textContent).toContain('connections.relayUnsupported');
    expect(button('connections.invite.create').disabled).toBe(true);
    expect(button('connections.accept.open').disabled).toBe(true);
  });

  it('asks to sign in when there is no account', async () => {
    mocks.overview.mockRejectedValue(new Error('not logged in'));
    await render();
    expect(container.textContent).toContain('connections.signedOut');
  });

  it('creates an invite and shows its link', async () => {
    mocks.createInvite.mockResolvedValue({
      inviteId: 'inv-1', link: 'openbitfun://connect?relay=x&code=C&fp=F', fingerprint: FINGERPRINT_A, expiresAt: 100,
    });
    await render();
    mocks.overview.mockResolvedValue(overview({ invites: [{ inviteId: 'inv-1', createdAt: 1, expiresAt: 100 }] }));
    await click('connections.invite.create');
    expect(container.querySelector('[data-testid="invite-link"]')?.textContent)
      .toBe('openbitfun://connect?relay=x&code=C&fp=F');
    // The new invite is listed once, with its link.
    expect(container.textContent).not.toContain('connections.invite.open');
    mocks.withdrawInvite.mockResolvedValue(undefined);
    mocks.overview.mockResolvedValue(overview());
    await click('connections.invite.withdraw');
    expect(mocks.withdrawInvite).toHaveBeenCalledWith('inv-1');
    expect(container.querySelector('[data-testid="invite-link"]')).toBeNull();
  });

  it('shows the inviter before accepting, then the fingerprint to read back', async () => {
    mocks.previewInvite.mockResolvedValue({
      inviterUserId: '1001', inviterDisplayName: 'Ann', inviterDeviceId: 'device-a',
      inviterDeviceName: 'A1 Desktop', fingerprint: FINGERPRINT_B, expiresAt: 100,
    });
    mocks.acceptInvite.mockResolvedValue(connection({ role: 'acceptor', status: 'pending' }));
    await render();
    await click('connections.accept.open');
    await type(' openbitfun://connect?relay=x&code=C&fp=F ');
    await click('connections.accept.check');
    expect(mocks.previewInvite).toHaveBeenCalledWith('openbitfun://connect?relay=x&code=C&fp=F');
    expect(container.textContent).toContain('connections.peer:Ann|1001');
    expect(container.querySelector('[data-testid="preview-fingerprint"]')?.textContent).toBe(FINGERPRINT_B);
    expect(mocks.acceptInvite).not.toHaveBeenCalled();
    await click('connections.accept.submit');
    expect(mocks.acceptInvite).toHaveBeenCalledWith('openbitfun://connect?relay=x&code=C&fp=F');
    expect(container.textContent).toContain('connections.accept.done');
    expect(container.textContent).toContain(FINGERPRINT_A);
  });

  it('explains a fingerprint mismatch and does not accept', async () => {
    mocks.previewInvite.mockRejectedValue(new Error('fingerprint_mismatch'));
    await render();
    await click('connections.accept.open');
    await type('openbitfun://connect?relay=x&code=C&fp=F');
    await click('connections.accept.check');
    expect(container.querySelector('[role="alert"]')?.textContent).toBe('connections.errors.fingerprint_mismatch');
    expect(mocks.acceptInvite).not.toHaveBeenCalled();
  });

  it('confirms a pending connection with the fingerprint the owner checked', async () => {
    mocks.overview.mockResolvedValue(overview({
      connections: [connection({ status: 'pending', usable: false, awaitingConfirmation: true })],
    }));
    mocks.confirm.mockResolvedValue(connection());
    await render();
    expect(container.textContent).toContain('connections.status.awaitingYou');
    await click('connections.confirm.open');
    expect(container.querySelector('[data-testid="confirm-fingerprint"]')?.textContent).toBe(FINGERPRINT_B);
    await click('connections.confirm.submit');
    expect(mocks.confirm).toHaveBeenCalledWith('conn-1', FINGERPRINT_B);
  });

  it('revokes, shows key changes and unreadable records, and hides revoke when revoked', async () => {
    mocks.overview.mockResolvedValue(overview({
      connections: [
        connection(),
        connection({ connectionId: 'conn-2', usable: false, unusableReason: 'key_changed' }),
        connection({ connectionId: 'conn-3', status: 'revoked', usable: false }),
        connection({ connectionId: 'conn-4', usable: false, unusableReason: 'not_verified' }),
      ],
      unreadable: [{ connectionId: 'conn-bad', error: 'missing field' }],
    }));
    mocks.revoke.mockResolvedValue(undefined);
    await render();
    expect(container.textContent).toContain('connections.status.keyChanged');
    expect(container.textContent).toContain('connections.status.notVerified');
    expect(container.textContent).toContain('connections.unreadable:missing field');
    const revokeButtons = [...container.querySelectorAll('button')]
      .filter(item => item.textContent === 'connections.revoke');
    expect(revokeButtons).toHaveLength(3);
    // A working connection names no status or fingerprint, only the assistants it can use.
    expect(container.querySelector('[data-row="connections.peerDevice:Bea|B1 Desktop"] small')!.textContent)
      .toBe('connections.noAssistants');
    await act(async () => { revokeButtons[0].click(); });
    expect(mocks.revoke).not.toHaveBeenCalled();
    expect(container.textContent).toContain('connections.revokeConfirmDescription');
    await act(async () => {
      container.querySelector('[data-row="connections.revokeConfirmTitle:Bea"] button:last-child')!
        .dispatchEvent(new MouseEvent('click', { bubbles: true }));
    });
    expect(mocks.revoke).toHaveBeenCalledWith('conn-1');
    expect(mocks.overview).toHaveBeenCalledTimes(2);
  });
  it('trusts a connection normally until the owner confirms full trust, and can return to normal', async () => {
    mocks.overview.mockResolvedValue(overview({ connections: [connection()] }));
    mocks.setTrust.mockResolvedValue(undefined);
    await render();
    const level = () => (container.querySelector('select[aria-label="connections.trust.label"]') as HTMLSelectElement).value;
    expect(level()).toBe('normal');
    await chooseTrust('full');
    expect(container.textContent).toContain('connections.trust.confirmDescription');
    expect(mocks.setTrust).not.toHaveBeenCalled();

    mocks.listTrust.mockResolvedValue({ 'conn-1': 'full' });
    await click('connections.trust.confirm');
    expect(mocks.setTrust).toHaveBeenCalledWith('conn-1', 'full', true);
    expect(level()).toBe('full');
    expect(container.textContent).not.toContain('connections.trust.confirmDescription');

    mocks.listTrust.mockResolvedValue({});
    await chooseTrust('normal');
    expect(mocks.setTrust).toHaveBeenLastCalledWith('conn-1', 'normal', false);
    expect(level()).toBe('normal');
  });

  it('names the opened assistants each account can use, without choosing them here', async () => {
    mocks.overview.mockResolvedValue(overview({ connections: [connection()] }));
    const agent = (id: string, name: string, connections: string[]) => ({ agent: {
      id, agentType: `assistant:${id}`, displayName: name, taskKinds: ['message'], sameAccountDevices: [],
      connections: connections.map(connectionId => ({ connectionId })),
    } });
    mocks.listOpenAgents.mockResolvedValue([
      agent('workspace', 'Nova', ['conn-1']),
      agent('workspace-work', 'Atlas', []),
      // Entries that are not assistants are never named.
      { agent: { id: 'coder', agentType: 'acp:claude-code', displayName: 'Coder', taskKinds: [], sameAccountDevices: [],
        connections: [{ connectionId: 'conn-1' }] } },
    ]);
    await render();
    expect(container.querySelector('[data-row="connections.peerDevice:Bea|B1 Desktop"] small')!.textContent)
      .toBe('connections.assistantsLabel:Nova');
    // Assistants are opened to a device on the assistant's own card.
    expect(container.textContent).not.toContain('connections.chooseAssistants');
  });

  it('reads every connection as normally trusted when trust cannot be read', async () => {
    mocks.overview.mockResolvedValue(overview({ connections: [connection()] }));
    mocks.listTrust.mockRejectedValue(new Error('unavailable'));
    await render();
    expect((container.querySelector('select[aria-label="connections.trust.label"]') as HTMLSelectElement).value)
      .toBe('normal');
  });
});
