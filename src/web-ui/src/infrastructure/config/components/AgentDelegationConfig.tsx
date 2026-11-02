import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Button, Checkbox, Input } from '@openbitfun/ui';
import {
  ALL_SAME_ACCOUNT_DEVICES,
  ASSISTANT_AGENT_TYPE_PREFIX,
  delegationAPI,
  MAX_CONNECTION_CONCURRENT_JOBS,
  MAX_CONNECTION_JOB_MINUTES,
  type ConnectionGrant,
  type ConnectionLimits,
  type DelegatedTask,
  type LocalAssistant,
  type OpenAgent,
} from '@/infrastructure/api/service-api/DelegationAPI';
import {
  connectionsAPI,
  type ConnectionSummary,
} from '@/infrastructure/api/service-api/ConnectionsAPI';
import {
  remoteConnectAPI,
  type AccountDeviceInfo,
} from '@/infrastructure/api/service-api/RemoteConnectAPI';
import { getWorkspaceDisplayName, useOptionalWorkspaceContext } from '@/infrastructure/contexts/WorkspaceContext';
import {
  notifyDelegationGrantsChanged,
  onDelegationGrantsChanged,
} from '@/infrastructure/api/service-api/delegationGrantChanges';
import {
  DELEGATED_TASKS_REFRESH_MS,
  readDelegatedTasks,
} from '@/infrastructure/api/service-api/delegatedTaskReads';
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

/**
 * Opening one assistant of this device to devices of other accounts. Each
 * chosen device gets its own grant.
 */
interface OpenDraft {
  agentType: string;
  connectionIds: string[];
  /** The owner confirmed that tasks may use external agents with this device's login. */
  usageConfirmed: boolean;
}

interface LoadedState {
  assistants: LocalAssistant[];
  agents: OpenAgent[];
  devices: AccountDeviceInfo[];
  /** Connections with devices of other accounts, revoked ones excluded. */
  connections: ConnectionSummary[];
  tasks: DelegatedTask[];
}

/** Limits as typed; empty means no limit. */
interface LimitsDraft {
  maxJobs: string;
  maxMinutes: string;
}

/** Granting one opened assistant to more devices of other accounts, each on its own. */
interface GrantDraft {
  agentId: string;
  connectionIds: string[];
}

/** Changing the limits of an existing grant. */
interface EditLimitsDraft extends LimitsDraft {
  agentId: string;
  connectionId: string;
}


/** How often tasks are read while one waits for the owner. */
const WAITING_TASKS_REFRESH_MS = 5_000;

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

/** The open-agent id other devices call an assistant by, from its directory. */
function openAgentIdFor(agentType: string): string {
  return agentType.slice(ASSISTANT_AGENT_TYPE_PREFIX.length).toLowerCase()
    .replace(/[^a-z0-9_-]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 64) || 'assistant';
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * The owner's assistants as other accounts can use them: which assistants are
 * open, to which devices, and how many tasks they sent. Each task is its own
 * session under the assistant, where the owner reads and approves it. The list is
 * device-local, so it is managed only on the device that owns it: hosts
 * without the desktop commands and connected peer devices show an explicit
 * state.
 */
export const AgentDelegationConfig: React.FC = () => {
  const { t } = useI18n('settings/runtime');
  const peerMode = usePeerDeviceModeOptional()?.peerMode;
  const workspaces = useOptionalWorkspaceContext();
  const supported = isTauriRuntime() && peerMode?.active !== true;
  const [state, setState] = useState<LoadedState | null>(null);
  const [loadError, setLoadError] = useState(false);
  const [draft, setDraft] = useState<OpenDraft | null>(null);
  const [draftError, setDraftError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [grantDraft, setGrantDraft] = useState<GrantDraft | null>(null);
  const [limitsDraft, setLimitsDraft] = useState<EditLimitsDraft | null>(null);
  /** The opened assistant whose closing the owner is confirming. */
  const [closing, setClosing] = useState<string | null>(null);
  const activeRef = useRef(true);

  const load = useCallback(async () => {
    setLoadError(false);
    try {
      const [assistants, agents, devices, currentDevice, connections, tasks] = await Promise.all([
        delegationAPI.listAssistants(),
        delegationAPI.listOpenAgents(),
        // Devices only name the devices of this account an assistant is opened to.
        remoteConnectAPI.accountListDevices().catch(() => [] as AccountDeviceInfo[]),
        remoteConnectAPI.getDeviceInfo().catch(() => null),
        // Connections are optional: without them there is nobody to open an assistant to.
        connectionsAPI.overview()
          .then(overview => overview.connections.filter(connection => connection.status !== 'revoked'))
          .catch(() => [] as ConnectionSummary[]),
        // Tasks are optional: without the dispatch runner nothing could have been delegated.
        readDelegatedTasks({ fresh: true }),
      ]);
      if (!activeRef.current) return;
      setState({
        assistants,
        agents,
        devices: devices.filter(device => device.device_id !== currentDevice?.device_id),
        connections,
        tasks,
      });
    } catch (error) {
      log.error('Failed to load opened assistants', { error });
      if (activeRef.current) setLoadError(true);
    }
  }, []);

  useEffect(() => {
    activeRef.current = true;
    if (supported) void load();
    return () => { activeRef.current = false; };
  }, [load, supported]);

  // Accounts can be given assistants from their own rows too.
  useEffect(() => onDelegationGrantsChanged(source => {
    if (source !== activeRef && supported) void load();
  }), [load, supported]);

  // Tasks arrive while the page stays open, so their counts are read again on
  // a timer, and promptly while one waits for the owner.
  const anyWaiting = state?.tasks.some(task => task.pendingPermissionCount > 0) ?? false;
  useEffect(() => {
    if (!supported) return undefined;
    const timer = setInterval(() => {
      void readDelegatedTasks({ fresh: anyWaiting }).then(tasks => {
        if (activeRef.current) setState(current => (current ? { ...current, tasks } : current));
      });
    }, anyWaiting ? WAITING_TASKS_REFRESH_MS : DELEGATED_TASKS_REFRESH_MS);
    return () => clearInterval(timer);
  }, [anyWaiting, supported]);

  const assistantName = (assistant: LocalAssistant): string => {
    const workspace = workspaces?.assistantWorkspacesList.find(item =>
      (item.assistantId ?? null) === assistant.assistantId);
    const name = workspace ? getWorkspaceDisplayName(workspace) : '';
    if (name) return name;
    return assistant.assistantId ?? t('agentDelegation.defaultAssistant');
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

  const describeDevices = (agent: OpenAgent): string => {
    if (agent.sameAccountDevices.includes(ALL_SAME_ACCOUNT_DEVICES)) return t('agentDelegation.allDevices');
    return agent.sameAccountDevices
      .map(id => state?.devices.find(device => device.device_id === id)?.device_name ?? id)
      .join(', ');
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

  /** Runs an owner action, reports its failure, and reloads the page. */
  const run = async (action: () => Promise<void>, failureLog: string, failureCopy: string) => {
    setBusy(true);
    try {
      await action();
      notifyDelegationGrantsChanged(activeRef);
      if (activeRef.current) await load();
    } catch (error) {
      log.error(failureLog, { error });
      notificationService.error(`${failureCopy}: ${errorMessage(error)}`);
    } finally {
      if (activeRef.current) setBusy(false);
    }
  };

  const saveDraft = async () => {
    if (!draft || !state) return;
    const assistant = state.assistants.find(item => item.agentType === draft.agentType);
    if (!assistant) return;
    setDraftError(null);
    setBusy(true);
    try {
      await delegationAPI.openAgent({
        id: openAgentIdFor(draft.agentType),
        agentType: draft.agentType,
        displayName: assistantName(assistant),
        description: null,
        sameAccountDevices: [],
        usageConfirmed: draft.usageConfirmed,
      });
      for (const connectionId of draft.connectionIds) {
        await delegationAPI.grantConnection(openAgentIdFor(draft.agentType), connectionId);
      }
      if (!activeRef.current) return;
      notificationService.success(t('agentDelegation.saved'), { duration: 2000 });
      notifyDelegationGrantsChanged(activeRef);
      setDraft(null);
      await load();
    } catch (error) {
      log.error('Failed to open assistant', { error });
      if (activeRef.current) setDraftError(`${t('agentDelegation.saveFailed')}: ${errorMessage(error)}`);
    } finally {
      if (activeRef.current) setBusy(false);
    }
  };

  const grant = () => {
    if (!grantDraft) return;
    void run(async () => {
      for (const connectionId of grantDraft.connectionIds) {
        await delegationAPI.grantConnection(grantDraft.agentId, connectionId);
      }
      notificationService.success(t('agentDelegation.granted'), { duration: 2000 });
      if (activeRef.current) setGrantDraft(null);
    }, 'Failed to grant assistant to a connection', t('agentDelegation.grantFailed'));
  };

  const saveLimits = () => {
    if (!limitsDraft) return;
    const limits = draftLimits(limitsDraft);
    if (!limits) return;
    void run(async () => {
      await delegationAPI.setConnectionLimits(limitsDraft.agentId, limitsDraft.connectionId, limits);
      if (activeRef.current) setLimitsDraft(null);
    }, 'Failed to set connection limits', t('agentDelegation.limits.saveFailed'));
  };

  /** Inputs for the two optional limits of one grant. */
  const renderLimitInputs = (value: LimitsDraft, onChange: (next: LimitsDraft) => void) => {
    const invalid = draftLimits(value) === null;
    return (
      <ConfigPageRow balanced label={t('agentDelegation.limits.title')}
        description={invalid ? t('agentDelegation.limits.invalid') : t('agentDelegation.limits.description')}
        align="center">
        <div className="openbitfun-runtime-settings__status-actions">
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

  /** Connections an assistant can still be granted to: usable and not granted yet. */
  const grantCandidates = (granted: string[]): ConnectionSummary[] => (state?.connections ?? [])
    .filter(connection => connection.usable && !granted.includes(connection.connectionId));

  /** One checkbox per device of another account; each checked device gets its own grant. */
  const renderDeviceChoice = (
    candidates: ConnectionSummary[],
    chosen: string[],
    onChange: (connectionIds: string[]) => void,
  ) => (
    <ConfigPageRow balanced label={t('agentDelegation.grantTitle')}
      description={candidates.length === 0 ? t('agentDelegation.noConnections') : t('agentDelegation.grantDescription')}>
      {candidates.length > 0 ? (
        <div className="openbitfun-runtime-settings__choice-list">
          {candidates.map(connection => (
            <Checkbox key={connection.connectionId} disabled={busy}
              checked={chosen.includes(connection.connectionId)}
              label={describeConnection(connection.connectionId)}
              onChange={event => onChange(event.target.checked
                ? [...chosen.filter(id => id !== connection.connectionId), connection.connectionId]
                : chosen.filter(id => id !== connection.connectionId))} />
          ))}
        </div>
      ) : <span />}
    </ConfigPageRow>
  );

  const renderGrantPanel = (agentId: string, granted: string[]) => {
    if (!state || grantDraft?.agentId !== agentId) return null;
    const candidates = grantCandidates(granted);
    return <>
      {renderDeviceChoice(candidates, grantDraft.connectionIds,
        connectionIds => setGrantDraft(current => current && ({ ...current, connectionIds })))}
      <ConfigPageRow label="" align="center" wide>
        <div className="openbitfun-runtime-settings__status-actions">
          <Button type="button" size="sm" variant="outline" disabled={busy} onClick={() => setGrantDraft(null)}>
            {t('agentDelegation.cancel')}
          </Button>
          {candidates.length > 0 && (
            <Button type="button" size="sm" variant="primary"
              disabled={busy || grantDraft.connectionIds.length === 0} onClick={grant}>
              {t('agentDelegation.grant')}
            </Button>
          )}
        </div>
      </ConfigPageRow>
    </>;
  };

  const renderGrants = (agent: OpenAgent) => (agent.connections ?? []).map(item => {
    const editing = limitsDraft?.agentId === agent.id && limitsDraft.connectionId === item.connectionId;
    return (
      <React.Fragment key={item.connectionId}>
        <ConfigPageRow balanced label={describeConnection(item.connectionId)}
          description={t('agentDelegation.grantedWith', { limits: describeLimits(item) })}
          align="center">
          <div className="openbitfun-runtime-settings__status-actions">
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
              onClick={() => void run(async () => { await delegationAPI.revokeConnection(agent.id, item.connectionId); },
                'Failed to withdraw assistant from a connection', t('agentDelegation.withdrawFailed'))}>
              {t('agentDelegation.withdraw')}
            </Button>
          </div>
        </ConfigPageRow>
        {editing && limitsDraft && <>
          {renderLimitInputs(limitsDraft,
            next => setLimitsDraft(current => current && ({ ...current, ...next })))}
          <ConfigPageRow label="" align="center" wide>
            <div className="openbitfun-runtime-settings__status-actions">
              <Button type="button" size="sm" variant="outline" disabled={busy}
                onClick={() => setLimitsDraft(null)}>
                {t('agentDelegation.cancel')}
              </Button>
              <Button type="button" size="sm" variant="primary"
                disabled={busy || draftLimits(limitsDraft) === null}
                onClick={saveLimits}>
                {t('agentDelegation.limits.save')}
              </Button>
            </div>
          </ConfigPageRow>
        </>}
      </React.Fragment>
    );
  });

  const renderOpenForm = (assistant: LocalAssistant) => {
    if (!state || draft?.agentType !== assistant.agentType) return null;
    const candidates = grantCandidates([]);
    const canSave = draft.connectionIds.length > 0 && draft.usageConfirmed;
    return <>
      {renderDeviceChoice(candidates, draft.connectionIds,
        connectionIds => setDraft(current => current && ({ ...current, connectionIds })))}
      {candidates.length > 0 && (
        <ConfigPageRow label={t('agentDelegation.usageNoticeTitle')} description={t('agentDelegation.usageNotice')}>
          <Checkbox checked={draft.usageConfirmed} disabled={busy}
            label={t('agentDelegation.usageConfirm')}
            onChange={event => setDraft({ ...draft, usageConfirmed: event.target.checked })} />
        </ConfigPageRow>
      )}
      {draftError && <div role="alert">{draftError}</div>}
      <ConfigPageRow label="" align="center" wide>
        <div className="openbitfun-runtime-settings__status-actions">
          <Button type="button" size="sm" variant="outline" disabled={busy}
            onClick={() => { setDraft(null); setDraftError(null); }}>
            {t('agentDelegation.cancel')}
          </Button>
          {candidates.length > 0 && (
            <Button type="button" size="sm" variant="primary" disabled={busy || !canSave}
              onClick={() => void saveDraft()}>
              {t('agentDelegation.save')}
            </Button>
          )}
        </div>
      </ConfigPageRow>
    </>;
  };

  const renderCloseConfirm = (agent: OpenAgent, name: string) => (
    <ConfigPageRow label={t('agentDelegation.closeConfirmTitle', { name })}
      description={t('agentDelegation.closeConfirmDescription')} align="center">
      <div className="openbitfun-runtime-settings__status-actions">
        <Button type="button" size="sm" variant="outline" disabled={busy} onClick={() => setClosing(null)}>
          {t('agentDelegation.cancel')}
        </Button>
        <Button type="button" size="sm" variant="primary" disabled={busy}
          onClick={() => void run(async () => {
            await delegationAPI.closeAgent(agent.id);
            if (activeRef.current) setClosing(null);
          }, 'Failed to close opened assistant', t('agentDelegation.closeFailed'))}>
          {t('agentDelegation.close')}
        </Button>
      </div>
    </ConfigPageRow>
  );

  /** How many tasks an assistant ran for others, and how many wait for the owner. */
  const describeTasks = (assistant: LocalAssistant): string | null => {
    const own = (state?.tasks ?? []).filter(task => task.agentType === assistant.agentType);
    if (own.length === 0) return null;
    const waiting = own.filter(task => task.pendingPermissionCount > 0).length;
    return [
      t('agentDelegation.taskCount', { count: String(own.length) }),
      waiting > 0 ? t('agentDelegation.tasksWaiting', { count: String(waiting) }) : null,
    ].filter(Boolean).join(' · ');
  };

  /** One card per assistant: which devices can use it; its tasks are sessions under it. */
  const renderAssistant = (assistant: LocalAssistant) => {
    const agent = state?.agents.find(item => item.agentType === assistant.agentType);
    const granted = agent?.connections?.map(item => item.connectionId) ?? [];
    const name = assistantName(assistant);
    const description = [
      !agent
        ? t('agentDelegation.notOpened')
        : agent.sameAccountDevices.length === 0
          ? t('agentDelegation.openedToAccounts')
          : t('agentDelegation.openedTo', { devices: describeDevices(agent) }),
      describeTasks(assistant),
    ].filter(Boolean).join(' · ');
    const actions = (
      <div className="openbitfun-runtime-settings__status-actions">
        {agent && grantDraft?.agentId !== agent.id && (
          <Button type="button" size="sm" variant="outline" disabled={busy}
            onClick={() => setGrantDraft({ agentId: agent.id, connectionIds: [] })}>
            {t('agentDelegation.grantConnection')}
          </Button>
        )}
        {agent && closing !== agent.id && (
          <Button type="button" size="sm" variant="outline" disabled={busy}
            onClick={() => setClosing(agent.id)}>
            {t('agentDelegation.close')}
          </Button>
        )}
        {!agent && draft?.agentType !== assistant.agentType && (
          <Button type="button" size="sm" variant="outline" disabled={busy || draft !== null}
            onClick={() => setDraft({
              agentType: assistant.agentType,
              connectionIds: [],
              usageConfirmed: false,
            })}>
            {t('agentDelegation.open')}
          </Button>
        )}
      </div>
    );
    const body = [
      agent && closing === agent.id ? <React.Fragment key="close">{renderCloseConfirm(agent, name)}</React.Fragment> : null,
      draft?.agentType === assistant.agentType ? <React.Fragment key="open">{renderOpenForm(assistant)}</React.Fragment> : null,
      agent ? <React.Fragment key="grants">{renderGrants(agent)}</React.Fragment> : null,
      agent && grantDraft?.agentId === agent.id
        ? <React.Fragment key="grant">{renderGrantPanel(agent.id, granted)}</React.Fragment> : null,
    ].filter(Boolean);
    return (
      <ConfigPageSection key={assistant.agentType} title={name} description={description} extra={actions}
        data-testid="assistant-card">
        {body.length > 0 ? body : null}
      </ConfigPageSection>
    );
  };

  const renderNotice = (content: React.ReactNode) => (
    <ConfigPageSection title={t('agentDelegation.sectionTitle')} description={t('agentDelegation.sectionDescription')}>
      {content}
    </ConfigPageSection>
  );

  if (!supported) {
    return renderNotice(
      <ConfigPageRow label={t('agentDelegation.sectionTitle')} description={peerMode?.active
        ? t('agentDelegation.peerUnsupported')
        : t('agentDelegation.unsupported')}>
        <span />
      </ConfigPageRow>,
    );
  }
  if (loadError) {
    return renderNotice(
      <ConfigRetryState message={t('loading.failed')} retryLabel={t('loading.retry')} onRetry={() => void load()} />,
    );
  }
  if (!state) return renderNotice(<ConfigLoadingState label={t('loading.text')} />);
  if (state.assistants.length === 0) {
    return renderNotice(<ConfigPageRow label={t('agentDelegation.noAssistants')} align="center"><span /></ConfigPageRow>);
  }
  return <>{state.assistants.map(renderAssistant)}</>;
};

export default AgentDelegationConfig;
