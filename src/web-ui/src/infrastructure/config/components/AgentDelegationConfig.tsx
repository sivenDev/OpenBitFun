import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Button, Checkbox, Input, Select, Switch } from '@openbitfun/ui';
import { agentAPI, type ModeInfo } from '@/infrastructure/api/service-api/AgentAPI';
import { ACPClientAPI, type AcpClientInfo } from '@/infrastructure/api/service-api/ACPClientAPI';
import {
  ALL_SAME_ACCOUNT_DEVICES,
  delegationAPI,
  externalAgentClientId,
  MAX_CONNECTION_CONCURRENT_JOBS,
  MAX_CONNECTION_JOB_MINUTES,
  openAgentRecordId,
  type ConnectionGrant,
  type ConnectionLimits,
  type DelegationTaskKind,
  type OpenAgentRecord,
} from '@/infrastructure/api/service-api/DelegationAPI';
import {
  connectionsAPI,
  type ConnectionSummary,
} from '@/infrastructure/api/service-api/ConnectionsAPI';
import {
  remoteConnectAPI,
  type AccountDeviceInfo,
} from '@/infrastructure/api/service-api/RemoteConnectAPI';
import { useI18n } from '@/infrastructure/i18n/hooks/useI18n';
import { usePeerDeviceModeOptional } from '@/infrastructure/peer-device/peerDeviceContextState';
import { isTauriRuntime } from '@/infrastructure/runtime';
import { notificationService } from '@/shared/notification-system';
import { createLogger } from '@/shared/utils/logger';
import {
  ConfigLoadingState,
  ConfigPageRow,
  ConfigPageSection,
  ConfigRetryState,
} from './common';

const log = createLogger('AgentDelegationConfig');

const OPEN_AGENT_ID_PATTERN = /^[A-Za-z0-9_-]{1,64}$/;

interface OpenAgentDraft {
  id: string;
  agentType: string;
  displayName: string;
  description: string;
  taskKinds: DelegationTaskKind[];
  allDevices: boolean;
  devices: string[];
  messageWorkspace: string;
  /** The owner confirmed the external agent's usage notice. */
  usageConfirmed: boolean;
}

/** An agent that can be opened: a local mode or an enabled external agent. */
interface AgentChoice {
  id: string;
  name: string;
}

interface LoadedState {
  records: OpenAgentRecord[];
  /** Local modes, then enabled external agents as `acp:<id>`. */
  agents: AgentChoice[];
  devices: AccountDeviceInfo[];
  /** Connections with devices of other accounts, revoked ones excluded. */
  connections: ConnectionSummary[];
}

/** Limits as typed; empty means no limit. */
interface LimitsDraft {
  maxJobs: string;
  maxMinutes: string;
}

/** Granting one open agent to a connection. */
interface GrantDraft extends LimitsDraft {
  agentId: string;
  connectionId: string;
  /** The owner confirmed the external agent's usage notice for this grant. */
  usageConfirmed: boolean;
}

/** Changing the limits of an existing grant. */
interface EditLimitsDraft extends LimitsDraft {
  agentId: string;
  connectionId: string;
}

const NO_LIMITS: LimitsDraft = { maxJobs: '', maxMinutes: '' };

function parseLimit(value: string, max: number): number | undefined | null {
  const trimmed = value.trim();
  if (trimmed === '') return undefined;
  if (!/^\d+$/.test(trimmed)) return null;
  const parsed = Number(trimmed);
  return parsed >= 1 && parsed <= max ? parsed : null;
}

/** The limits a draft names, or null when one is out of range. */
function draftLimits(draft: LimitsDraft): ConnectionLimits | null {
  const maxConcurrentJobs = parseLimit(draft.maxJobs, MAX_CONNECTION_CONCURRENT_JOBS);
  const maxJobMinutes = parseLimit(draft.maxMinutes, MAX_CONNECTION_JOB_MINUTES);
  if (maxConcurrentJobs === null || maxJobMinutes === null) return null;
  return { maxConcurrentJobs, maxJobMinutes };
}

function hasLimits(limits: ConnectionLimits): boolean {
  return limits.maxConcurrentJobs !== undefined || limits.maxJobMinutes !== undefined;
}

function suggestedId(agentType: string): string {
  return agentType.toLowerCase().replace(/[^a-z0-9_-]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 64);
}

function agentChoices(modes: ModeInfo[], clients: AcpClientInfo[]): AgentChoice[] {
  // The device re-checks that an external agent can run before opening it.
  const external = clients
    .filter(client => client.enabled && client.subagent?.enabled !== false)
    .map(client => ({ id: `acp:${client.id}`, name: client.name || client.id }));
  return [...modes.map(mode => ({ id: mode.id, name: mode.name })), ...external];
}

function newDraft(agents: AgentChoice[]): OpenAgentDraft {
  const agentType = agents[0]?.id ?? '';
  return {
    id: suggestedId(agentType),
    agentType,
    displayName: agents[0]?.name ?? '',
    description: '',
    taskKinds: ['message'],
    allDevices: false,
    devices: [],
    messageWorkspace: '',
    usageConfirmed: false,
  };
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * Agents this device opens to other devices of the same account. The list is
 * device-local, so it is managed only on the device that owns it: hosts without
 * the desktop commands and connected peer devices show an explicit state.
 */
export const AgentDelegationConfig: React.FC = () => {
  const { t } = useI18n('settings/runtime');
  const peerMode = usePeerDeviceModeOptional()?.peerMode;
  const supported = isTauriRuntime() && peerMode?.active !== true;
  const [state, setState] = useState<LoadedState | null>(null);
  const [loadError, setLoadError] = useState(false);
  const [draft, setDraft] = useState<OpenAgentDraft | null>(null);
  const [draftError, setDraftError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [grantDraft, setGrantDraft] = useState<GrantDraft | null>(null);
  const [limitsDraft, setLimitsDraft] = useState<EditLimitsDraft | null>(null);
  const activeRef = useRef(true);

  const load = useCallback(async () => {
    setLoadError(false);
    try {
      const [records, modes, clients, devices, currentDevice, connections] = await Promise.all([
        delegationAPI.listOpenAgents(),
        agentAPI.getAvailableModes(),
        // External agents are optional: a host without ACP clients opens local modes only.
        ACPClientAPI.getClients().catch(() => [] as AcpClientInfo[]),
        // Devices are optional: without an account the owner can still allow all devices.
        remoteConnectAPI.accountListDevices().catch(() => [] as AccountDeviceInfo[]),
        remoteConnectAPI.getDeviceInfo().catch(() => null),
        // Connections are optional: without them only same-account devices are offered.
        connectionsAPI.overview()
          .then(overview => overview.connections.filter(connection => connection.status !== 'revoked'))
          .catch(() => [] as ConnectionSummary[]),
      ]);
      if (!activeRef.current) return;
      setState({
        records,
        agents: agentChoices(modes, clients),
        devices: devices.filter(device => device.device_id !== currentDevice?.device_id),
        connections,
      });
    } catch (error) {
      log.error('Failed to load open agents', { error });
      if (activeRef.current) setLoadError(true);
    }
  }, []);

  useEffect(() => {
    activeRef.current = true;
    if (supported) void load();
    return () => { activeRef.current = false; };
  }, [load, supported]);

  const describeRecord = (record: OpenAgentRecord): string => {
    const agent = record.agent;
    if (!agent) return t('agentDelegation.unreadable', { problem: record.problem ?? '' });
    const choice = state?.agents.find(item => item.id === agent.agentType);
    const tasks = agent.taskKinds
      .map(kind => kind === 'message'
        ? t('agentDelegation.taskKindMessage')
        : kind === 'coding' ? t('agentDelegation.taskKindCoding') : null)
      .filter((label): label is string => label !== null)
      .join(', ');
    const devices = agent.sameAccountDevices.includes(ALL_SAME_ACCOUNT_DEVICES)
      ? t('agentDelegation.allDevices')
      : agent.sameAccountDevices
        .map(id => state?.devices.find(device => device.device_id === id)?.device_name ?? id)
        .join(', ');
    return t('agentDelegation.summary', { agent: choice?.name ?? agent.agentType, tasks, devices });
  };

  const describeConnection = (connectionId: string): string => {
    const connection = state?.connections.find(item => item.connectionId === connectionId);
    return connection
      ? t('agentDelegation.connectionLabel', {
        name: connection.peerDisplayName,
        user: connection.peerUserId,
        device: connection.peerDeviceName,
      })
      : t('agentDelegation.connectionMissing', { id: connectionId });
  };

  const describeLimits = (grant: ConnectionGrant): string => {
    const jobs = grant.maxConcurrentJobs !== undefined
      ? t('agentDelegation.limits.jobs', { count: String(grant.maxConcurrentJobs) })
      : t('agentDelegation.limits.anyJobs');
    const minutes = grant.maxJobMinutes !== undefined
      ? t('agentDelegation.limits.minutes', { count: String(grant.maxJobMinutes) })
      : t('agentDelegation.limits.anyMinutes');
    return `${jobs} · ${minutes}`;
  };

  const grant = async () => {
    if (!grantDraft) return;
    const limits = draftLimits(grantDraft);
    if (!limits) return;
    setBusy(true);
    try {
      await delegationAPI.grantConnection(grantDraft.agentId, grantDraft.connectionId, grantDraft.usageConfirmed);
      if (hasLimits(limits)) {
        await delegationAPI.setConnectionLimits(grantDraft.agentId, grantDraft.connectionId, limits);
      }
      if (!activeRef.current) return;
      notificationService.success(t('agentDelegation.granted'), { duration: 2000 });
      setGrantDraft(null);
      await load();
    } catch (error) {
      log.error('Failed to grant open agent to a connection', { error });
      notificationService.error(`${t('agentDelegation.grantFailed')}: ${errorMessage(error)}`);
    } finally {
      if (activeRef.current) setBusy(false);
    }
  };

  const saveLimits = async () => {
    if (!limitsDraft) return;
    const limits = draftLimits(limitsDraft);
    if (!limits) return;
    setBusy(true);
    try {
      await delegationAPI.setConnectionLimits(limitsDraft.agentId, limitsDraft.connectionId, limits);
      if (!activeRef.current) return;
      setLimitsDraft(null);
      await load();
    } catch (error) {
      log.error('Failed to set connection limits', { error });
      notificationService.error(`${t('agentDelegation.limits.saveFailed')}: ${errorMessage(error)}`);
    } finally {
      if (activeRef.current) setBusy(false);
    }
  };

  /** Inputs for the two optional limits of one grant. */
  const renderLimitInputs = (value: LimitsDraft, onChange: (next: LimitsDraft) => void) => {
    const invalid = draftLimits(value) === null;
    return (
      <ConfigPageRow label={t('agentDelegation.limits.title')}
        description={invalid ? t('agentDelegation.limits.invalid') : t('agentDelegation.limits.description')}
        align="center">
        <div>
          <Input size="sm" inputMode="numeric" disabled={busy} value={value.maxJobs}
            aria-label={t('agentDelegation.limits.jobsLabel')}
            placeholder={t('agentDelegation.limits.jobsLabel')}
            onChange={event => onChange({ ...value, maxJobs: event.target.value })} />
          <Input size="sm" inputMode="numeric" disabled={busy} value={value.maxMinutes}
            aria-label={t('agentDelegation.limits.minutesLabel')}
            placeholder={t('agentDelegation.limits.minutesLabel')}
            onChange={event => onChange({ ...value, maxMinutes: event.target.value })} />
        </div>
      </ConfigPageRow>
    );
  };

  const withdraw = async (agentId: string, connectionId: string) => {
    setBusy(true);
    try {
      await delegationAPI.revokeConnection(agentId, connectionId);
      await load();
    } catch (error) {
      log.error('Failed to withdraw open agent from a connection', { error });
      notificationService.error(t('agentDelegation.withdrawFailed'));
    } finally {
      if (activeRef.current) setBusy(false);
    }
  };

  /** Connections an agent can still be granted to: usable and not granted yet. */
  const grantCandidates = (granted: string[]): ConnectionSummary[] => (state?.connections ?? [])
    .filter(connection => connection.usable && !granted.includes(connection.connectionId));

  const renderGrantPanel = (agentId: string, agentType: string, granted: string[]) => {
    if (!state || grantDraft?.agentId !== agentId) return null;
    const candidates = grantCandidates(granted);
    const external = externalAgentClientId(agentType) !== null;
    const canGrant = grantDraft.connectionId !== '' && (!external || grantDraft.usageConfirmed)
      && draftLimits(grantDraft) !== null;
    return <>
      <ConfigPageRow label={t('agentDelegation.grantTitle')}
        description={candidates.length === 0 ? t('agentDelegation.noConnections') : t('agentDelegation.grantDescription')}
        align="center">
        {candidates.length > 0 && (
          <Select
            size="sm"
            value={grantDraft.connectionId}
            disabled={busy}
            options={candidates.map(connection => ({
              value: connection.connectionId,
              label: describeConnection(connection.connectionId),
            }))}
            onValueChange={value => {
              const connectionId = String(Array.isArray(value) ? value[0] : value);
              // A confirmation covers only the account it was given for.
              setGrantDraft(current => current && ({ ...current, connectionId, usageConfirmed: false }));
            }}
          />
        )}
      </ConfigPageRow>
      {external && candidates.length > 0 && (
        <ConfigPageRow label={t('agentDelegation.usageNoticeTitle')}
          description={externalAgentClientId(agentType) === 'claude-code'
            ? `${t('agentDelegation.grantUsageNotice')} ${t('agentDelegation.usageNoticeClaudeCode')}`
            : t('agentDelegation.grantUsageNotice')}>
          <Checkbox checked={grantDraft.usageConfirmed} disabled={busy}
            label={t('agentDelegation.grantUsageConfirm')}
            onChange={event => setGrantDraft({ ...grantDraft, usageConfirmed: event.target.checked })} />
        </ConfigPageRow>
      )}
      {candidates.length > 0 && renderLimitInputs(grantDraft,
        next => setGrantDraft(current => current && ({ ...current, ...next })))}
      <ConfigPageRow label="" align="center">
        <div>
          <Button type="button" size="sm" variant="outline" disabled={busy} onClick={() => setGrantDraft(null)}>
            {t('agentDelegation.cancel')}
          </Button>
          <Button type="button" size="sm" variant="primary" disabled={busy || !canGrant} onClick={() => void grant()}>
            {t('agentDelegation.grant')}
          </Button>
        </div>
      </ConfigPageRow>
    </>;
  };

  const closeRecord = async (id: string) => {
    setBusy(true);
    try {
      await delegationAPI.closeAgent(id);
      await load();
    } catch (error) {
      log.error('Failed to close open agent', { error });
      notificationService.error(t('agentDelegation.closeFailed'));
    } finally {
      if (activeRef.current) setBusy(false);
    }
  };

  const saveDraft = async () => {
    if (!draft) return;
    const id = draft.id.trim();
    if (!OPEN_AGENT_ID_PATTERN.test(id)) {
      setDraftError(t('agentDelegation.idDescription'));
      return;
    }
    setDraftError(null);
    setBusy(true);
    try {
      await delegationAPI.openAgent({
        id,
        agentType: draft.agentType,
        displayName: draft.displayName.trim(),
        description: draft.description.trim() || null,
        taskKinds: draft.taskKinds,
        messageWorkspace: draft.messageWorkspace.trim() || null,
        sameAccountDevices: draft.allDevices ? [ALL_SAME_ACCOUNT_DEVICES] : draft.devices,
        ...(externalAgentClientId(draft.agentType) !== null ? { usageConfirmed: draft.usageConfirmed } : {}),
      });
      if (!activeRef.current) return;
      notificationService.success(t('agentDelegation.saved'), { duration: 2000 });
      setDraft(null);
      await load();
    } catch (error) {
      log.error('Failed to open agent', { error });
      if (activeRef.current) setDraftError(`${t('agentDelegation.saveFailed')}: ${errorMessage(error)}`);
    } finally {
      if (activeRef.current) setBusy(false);
    }
  };

  const toggleTaskKind = (kind: DelegationTaskKind, checked: boolean) => {
    setDraft(current => current && ({
      ...current,
      taskKinds: checked
        ? [...current.taskKinds.filter(item => item !== kind), kind]
        : current.taskKinds.filter(item => item !== kind),
    }));
  };

  const toggleDevice = (deviceId: string, checked: boolean) => {
    setDraft(current => current && ({
      ...current,
      devices: checked
        ? [...current.devices.filter(item => item !== deviceId), deviceId]
        : current.devices.filter(item => item !== deviceId),
    }));
  };

  const renderBody = () => {
    if (!supported) {
      return (
        <ConfigPageRow label={t('agentDelegation.sectionTitle')} description={peerMode?.active
          ? t('agentDelegation.peerUnsupported')
          : t('agentDelegation.unsupported')}>
          <span />
        </ConfigPageRow>
      );
    }
    if (loadError) {
      return <ConfigRetryState message={t('loading.failed')} retryLabel={t('loading.retry')} onRetry={() => void load()} />;
    }
    if (!state) return <ConfigLoadingState label={t('loading.text')} />;
    const externalClientId = draft ? externalAgentClientId(draft.agentType) : null;
    const canSave = draft !== null
      && draft.agentType !== ''
      && draft.taskKinds.length > 0
      && (draft.allDevices || draft.devices.length > 0)
      && (externalClientId === null || draft.usageConfirmed);
    return <>
      {state.records.length === 0 && !draft && (
        <ConfigPageRow label={t('agentDelegation.empty')} align="center"><span /></ConfigPageRow>
      )}
      {state.records.map((record, index) => {
        const id = openAgentRecordId(record);
        const agent = record.agent;
        const grants = agent?.connections ?? [];
        const granted = grants.map(item => item.connectionId);
        return (
          <React.Fragment key={id ?? `unreadable-${index}`}>
            <ConfigPageRow
              label={agent?.displayName ?? id ?? t('agentDelegation.unreadable', { problem: '' })}
              description={describeRecord(record)}
              align="center"
            >
              <div>
                {agent && grantDraft?.agentId !== agent.id && (
                  <Button type="button" size="sm" variant="outline" disabled={busy}
                    onClick={() => setGrantDraft({
                      agentId: agent.id,
                      connectionId: grantCandidates(granted)[0]?.connectionId ?? '',
                      usageConfirmed: false,
                      ...NO_LIMITS,
                    })}>
                    {t('agentDelegation.grantConnection')}
                  </Button>
                )}
                {id && (
                  <Button type="button" size="sm" variant="outline" disabled={busy} onClick={() => void closeRecord(id)}>
                    {t('agentDelegation.close')}
                  </Button>
                )}
              </div>
            </ConfigPageRow>
            {agent && grants.map(item => {
              const editing = limitsDraft?.agentId === agent.id && limitsDraft.connectionId === item.connectionId;
              return (
                <React.Fragment key={item.connectionId}>
                  <ConfigPageRow label={t('agentDelegation.otherAccounts')}
                    description={`${t('agentDelegation.grantedTo', { peer: describeConnection(item.connectionId) })} · ${describeLimits(item)}`}
                    align="center">
                    <div>
                      {!editing && (
                        <Button type="button" size="sm" variant="outline" disabled={busy}
                          onClick={() => setLimitsDraft({
                            agentId: agent.id,
                            connectionId: item.connectionId,
                            maxJobs: item.maxConcurrentJobs?.toString() ?? '',
                            maxMinutes: item.maxJobMinutes?.toString() ?? '',
                          })}>
                          {t('agentDelegation.limits.edit')}
                        </Button>
                      )}
                      <Button type="button" size="sm" variant="outline" disabled={busy}
                        onClick={() => void withdraw(agent.id, item.connectionId)}>
                        {t('agentDelegation.withdraw')}
                      </Button>
                    </div>
                  </ConfigPageRow>
                  {editing && limitsDraft && <>
                    {renderLimitInputs(limitsDraft,
                      next => setLimitsDraft(current => current && ({ ...current, ...next })))}
                    <ConfigPageRow label="" align="center">
                      <div>
                        <Button type="button" size="sm" variant="outline" disabled={busy}
                          onClick={() => setLimitsDraft(null)}>
                          {t('agentDelegation.cancel')}
                        </Button>
                        <Button type="button" size="sm" variant="primary"
                          disabled={busy || draftLimits(limitsDraft) === null}
                          onClick={() => void saveLimits()}>
                          {t('agentDelegation.limits.save')}
                        </Button>
                      </div>
                    </ConfigPageRow>
                  </>}
                </React.Fragment>
              );
            })}
            {agent && renderGrantPanel(agent.id, agent.agentType, granted)}
          </React.Fragment>
        );
      })}
      {draft ? <>
        <ConfigPageRow label={t('agentDelegation.mode')} align="center">
          <Select
            size="sm"
            value={draft.agentType}
            disabled={busy}
            options={state.agents.map(agent => ({
              value: agent.id,
              label: externalAgentClientId(agent.id) === null
                ? agent.name
                : t('agentDelegation.externalAgent', { name: agent.name }),
            }))}
            onValueChange={value => {
              const agentType = String(Array.isArray(value) ? value[0] : value);
              const choice = state.agents.find(item => item.id === agentType);
              setDraft(current => current && ({
                ...current,
                agentType,
                id: suggestedId(agentType),
                displayName: choice?.name ?? current.displayName,
                // A confirmation covers only the agent it was given for.
                usageConfirmed: false,
              }));
            }}
          />
        </ConfigPageRow>
        {externalClientId !== null && (
          <ConfigPageRow
            label={t('agentDelegation.usageNoticeTitle')}
            description={externalClientId === 'claude-code'
              ? `${t('agentDelegation.usageNotice')} ${t('agentDelegation.usageNoticeClaudeCode')}`
              : t('agentDelegation.usageNotice')}
          >
            <Checkbox checked={draft.usageConfirmed} disabled={busy}
              label={t('agentDelegation.usageConfirm')}
              onChange={event => setDraft({ ...draft, usageConfirmed: event.target.checked })} />
          </ConfigPageRow>
        )}
        <ConfigPageRow label={t('agentDelegation.id')} description={t('agentDelegation.idDescription')} align="center">
          <Input size="sm" value={draft.id} disabled={busy}
            onChange={event => setDraft({ ...draft, id: event.target.value })} />
        </ConfigPageRow>
        <ConfigPageRow label={t('agentDelegation.name')} align="center">
          <Input size="sm" value={draft.displayName} disabled={busy}
            onChange={event => setDraft({ ...draft, displayName: event.target.value })} />
        </ConfigPageRow>
        <ConfigPageRow label={t('agentDelegation.description')} align="center">
          <Input size="sm" value={draft.description} disabled={busy}
            onChange={event => setDraft({ ...draft, description: event.target.value })} />
        </ConfigPageRow>
        <ConfigPageRow label={t('agentDelegation.taskKinds')} description={t('agentDelegation.taskKindsDescription')}>
          <div>
            <Checkbox checked={draft.taskKinds.includes('message')} disabled={busy}
              label={t('agentDelegation.taskKindMessage')}
              onChange={event => toggleTaskKind('message', event.target.checked)} />
            <Checkbox checked={draft.taskKinds.includes('coding')} disabled={busy}
              label={t('agentDelegation.taskKindCoding')}
              onChange={event => toggleTaskKind('coding', event.target.checked)} />
          </div>
        </ConfigPageRow>
        <ConfigPageRow label={t('agentDelegation.allDevices')} align="center">
          <Switch checked={draft.allDevices} disabled={busy}
            onChange={event => setDraft({ ...draft, allDevices: event.target.checked })} />
        </ConfigPageRow>
        {!draft.allDevices && (
          <ConfigPageRow label={t('agentDelegation.devices')}
            description={state.devices.length === 0 ? t('agentDelegation.noDevices') : undefined}>
            <div>
              {state.devices.map(device => (
                <Checkbox key={device.device_id} checked={draft.devices.includes(device.device_id)} disabled={busy}
                  label={device.device_name || device.device_id}
                  onChange={event => toggleDevice(device.device_id, event.target.checked)} />
              ))}
            </div>
          </ConfigPageRow>
        )}
        <ConfigPageRow label={t('agentDelegation.workspace')} description={t('agentDelegation.workspaceDescription')} align="center">
          <Input size="sm" value={draft.messageWorkspace} disabled={busy}
            onChange={event => setDraft({ ...draft, messageWorkspace: event.target.value })} />
        </ConfigPageRow>
        {draftError && <div role="alert">{draftError}</div>}
        <ConfigPageRow label="" align="center">
          <div>
            <Button type="button" size="sm" variant="outline" disabled={busy}
              onClick={() => { setDraft(null); setDraftError(null); }}>
              {t('agentDelegation.cancel')}
            </Button>
            <Button type="button" size="sm" variant="primary" disabled={busy || !canSave} onClick={() => void saveDraft()}>
              {t('agentDelegation.save')}
            </Button>
          </div>
        </ConfigPageRow>
      </> : (
        <ConfigPageRow label="" align="center">
          <Button type="button" size="sm" variant="outline" disabled={busy || state.agents.length === 0}
            onClick={() => setDraft(newDraft(state.agents))}>
            {t('agentDelegation.open')}
          </Button>
        </ConfigPageRow>
      )}
    </>;
  };

  return (
    <ConfigPageSection
      title={t('agentDelegation.sectionTitle')}
      description={t('agentDelegation.sectionDescription')}
    >
      {renderBody()}
    </ConfigPageSection>
  );
};

export default AgentDelegationConfig;
