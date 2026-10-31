import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Button, Checkbox, Input, Select, Switch } from '@openbitfun/ui';
import {
  ALL_SAME_ACCOUNT_DEVICES,
  ASSISTANT_AGENT_TYPE_PREFIX,
  delegationAPI,
  isAssistantAgentType,
  MAX_CONNECTION_CONCURRENT_JOBS,
  MAX_CONNECTION_JOB_MINUTES,
  openAgentRecordId,
  type ConnectionGrant,
  type ConnectionLimits,
  type DelegatedTask,
  type DelegatedTaskStatus,
  type LocalAssistant,
  type OpenAgent,
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
import { getWorkspaceDisplayName, useOptionalWorkspaceContext } from '@/infrastructure/contexts/WorkspaceContext';
import {
  DELEGATED_TASKS_REFRESH_MS,
  readDelegatedTasks,
} from '@/infrastructure/api/service-api/delegatedTaskReads';
import { useI18n } from '@/infrastructure/i18n/hooks/useI18n';
import { permissionActionLabel } from '@/shared/utils/permissionActionLabels';
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

/** Opening one assistant to devices of this account. */
interface OpenDraft {
  agentType: string;
  allDevices: boolean;
  devices: string[];
  /** The owner confirmed that tasks may use external agents with this device's login. */
  usageConfirmed: boolean;
}

interface LoadedState {
  assistants: LocalAssistant[];
  records: OpenAgentRecord[];
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

/** Granting one opened assistant to a connection. */
interface GrantDraft extends LimitsDraft {
  agentId: string;
  connectionId: string;
}

/** Changing the limits of an existing grant. */
interface EditLimitsDraft extends LimitsDraft {
  agentId: string;
  connectionId: string;
}

const NO_LIMITS: LimitsDraft = { maxJobs: '', maxMinutes: '' };

/** Task states after which nothing more happens. */
const FINISHED_TASK_STATES = new Set(['succeeded', 'failed', 'cancelled']);

/** How often tasks are read while one waits for the owner. */
const WAITING_TASKS_REFRESH_MS = 5_000;

/** Finished tasks still listed under an assistant; older ones are in the record. */
const RECENT_FINISHED_TASKS = 3;

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

/** The open-agent id other devices call an assistant by, from its directory. */
function openAgentIdFor(agentType: string): string {
  return agentType.slice(ASSISTANT_AGENT_TYPE_PREFIX.length).toLowerCase()
    .replace(/[^a-z0-9_-]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 64) || 'assistant';
}

/**
 * A prompt's resource, relative to the assistant's folder when it lies inside
 * it. The runner may report the folder through a symlink-resolved path
 * (`/private/var` for `/var` on macOS), so both spellings are accepted.
 */
function assistantRelative(resource: string, root: string): string | null {
  const normalize = (path: string) => path.replace(/[\\/]+$/, '').replace(/^\/private(?=\/)/, '');
  const base = normalize(root);
  const path = normalize(resource);
  if (path === base) return '';
  if (path.startsWith(`${base}/`) || path.startsWith(`${base}\\`)) return path.slice(base.length + 1);
  return null;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * The owner's assistants as other devices and accounts can use them: which
 * assistants are open, to whom, and the tasks they sent, each its own session
 * of that assistant that the owner reviews and approves here. The list is
 * device-local, so it is managed only on the device that owns it: hosts
 * without the desktop commands and connected peer devices show an explicit
 * state.
 */
export const AgentDelegationConfig: React.FC = () => {
  const { t } = useI18n('settings/runtime');
  const { t: tChat } = useI18n('flow-chat');
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
  /** Prompts of the tasks that wait for the owner, by job. */
  const [statuses, setStatuses] = useState<Record<string, DelegatedTaskStatus>>({});
  /** Prompts answered here (`jobId:requestId`); the runner may list them until it consumes the answer. */
  const [answered, setAnswered] = useState<ReadonlySet<string>>(new Set());
  /** The opened assistant whose closing the owner is confirming. */
  const [closing, setClosing] = useState<string | null>(null);
  const activeRef = useRef(true);

  const load = useCallback(async () => {
    setLoadError(false);
    try {
      const [assistants, records, devices, currentDevice, connections, tasks] = await Promise.all([
        delegationAPI.listAssistants(),
        delegationAPI.listOpenAgents(),
        // Devices are optional: without an account the owner can still allow all devices.
        remoteConnectAPI.accountListDevices().catch(() => [] as AccountDeviceInfo[]),
        remoteConnectAPI.getDeviceInfo().catch(() => null),
        // Connections are optional: without them only same-account devices are offered.
        connectionsAPI.overview()
          .then(overview => overview.connections.filter(connection => connection.status !== 'revoked'))
          .catch(() => [] as ConnectionSummary[]),
        // Tasks are optional: without the dispatch runner nothing could have been delegated.
        readDelegatedTasks({ fresh: true }),
      ]);
      if (!activeRef.current) return;
      setState({
        assistants,
        records,
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

  // Tasks arrive while the page stays open, so they are read again on a timer,
  // and promptly while one waits for the owner.
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

  const describeSource = (task: DelegatedTask): string => {
    const { source } = task;
    if (source.kind === 'connection') {
      return source.accountName
        ? t('agentDelegation.tasks.fromAccount', {
          name: source.accountName,
          device: source.deviceName ?? source.deviceId ?? '',
        })
        : t('agentDelegation.tasks.fromConnection', { id: source.connectionId ?? '' });
    }
    return t('agentDelegation.tasks.fromDevice', {
      device: source.deviceName ?? source.deviceId ?? t('agentDelegation.tasks.unknownDevice'),
    });
  };

  const describeState = (taskState: string): string => {
    switch (taskState) {
      case 'queued': return t('agentDelegation.tasks.state.queued');
      case 'running': return t('shared:statuses.running');
      case 'succeeded': return t('shared:statuses.done');
      case 'failed': return t('shared:statuses.failed');
      case 'cancelled': return t('shared:statuses.cancelled');
      default: return taskState;
    }
  };

  /** Runs an owner action, reports its failure, and reloads the page. */
  const run = async (action: () => Promise<void>, failureLog: string, failureCopy: string) => {
    setBusy(true);
    try {
      await action();
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
        sameAccountDevices: draft.allDevices ? [ALL_SAME_ACCOUNT_DEVICES] : draft.devices,
        usageConfirmed: draft.usageConfirmed,
      });
      if (!activeRef.current) return;
      notificationService.success(t('agentDelegation.saved'), { duration: 2000 });
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
    const limits = draftLimits(grantDraft);
    if (!limits) return;
    void run(async () => {
      await delegationAPI.grantConnection(grantDraft.agentId, grantDraft.connectionId);
      if (hasLimits(limits)) {
        await delegationAPI.setConnectionLimits(grantDraft.agentId, grantDraft.connectionId, limits);
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

  /** Reads the prompts of every task that waits for the owner. */
  const loadStatuses = useCallback(async (tasks: DelegatedTask[]) => {
    const waiting = tasks.filter(task => task.pendingPermissionCount > 0);
    const loaded = await Promise.all(waiting.map(task => delegationAPI.taskStatus(task.jobId)
      .then(status => [task.jobId, status] as const)
      .catch((error: unknown) => {
        log.warn('Failed to load delegated task', { error });
        return null;
      })));
    if (!activeRef.current) return;
    setStatuses(Object.fromEntries(loaded.filter(entry => entry !== null)));
  }, []);

  const tasks = state?.tasks;
  useEffect(() => {
    if (tasks) void loadStatuses(tasks);
  }, [loadStatuses, tasks]);

  const answer = (jobId: string, requestId: string, reply: 'once' | 'reject') => void run(async () => {
    await delegationAPI.answerTask(jobId, requestId, reply);
    if (activeRef.current) setAnswered(current => new Set(current).add(`${jobId}:${requestId}`));
  }, 'Failed to answer delegated task prompt', t('agentDelegation.tasks.answerFailed'));

  /** What a task still waits for: its read prompts without those answered here. */
  const openPrompts = (task: DelegatedTask) => (statuses[task.jobId]?.pendingPermissions ?? [])
    .filter(prompt => !answered.has(`${task.jobId}:${prompt.requestId}`));

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

  const renderGrantPanel = (agentId: string, granted: string[]) => {
    if (!state || grantDraft?.agentId !== agentId) return null;
    const candidates = grantCandidates(granted);
    const canGrant = grantDraft.connectionId !== '' && draftLimits(grantDraft) !== null;
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
              setGrantDraft(current => current && ({ ...current, connectionId }));
            }}
          />
        )}
      </ConfigPageRow>
      {candidates.length > 0 && renderLimitInputs(grantDraft,
        next => setGrantDraft(current => current && ({ ...current, ...next })))}
      <ConfigPageRow label="" align="center" wide>
        <div className="openbitfun-runtime-settings__status-actions">
          <Button type="button" size="sm" variant="outline" disabled={busy} onClick={() => setGrantDraft(null)}>
            {t('agentDelegation.cancel')}
          </Button>
          <Button type="button" size="sm" variant="primary" disabled={busy || !canGrant} onClick={grant}>
            {t('agentDelegation.grant')}
          </Button>
        </div>
      </ConfigPageRow>
    </>;
  };

  const renderGrants = (agent: OpenAgent) => (agent.connections ?? []).map(item => {
    const editing = limitsDraft?.agentId === agent.id && limitsDraft.connectionId === item.connectionId;
    return (
      <React.Fragment key={item.connectionId}>
        <ConfigPageRow balanced label={describeConnection(item.connectionId)}
          description={describeLimits(item)}
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

  const describeResource = (resource: string, root: string): string => {
    const relative = assistantRelative(resource, root);
    if (relative === null) return resource;
    return relative === '' ? t('agentDelegation.tasks.assistantFolder') : relative;
  };

  const renderPrompts = (task: DelegatedTask, root: string) => openPrompts(task).map(prompt => (
    <ConfigPageRow key={prompt.requestId} balanced
      label={permissionActionLabel(prompt.action, tChat)}
      description={prompt.resources.map(resource => describeResource(resource, root)).join('\n')}
      align="center">
      <div className="openbitfun-runtime-settings__status-actions">
        <Button type="button" size="sm" variant="outline" disabled={busy}
          onClick={() => answer(task.jobId, prompt.requestId, 'reject')}>
          {t('agentDelegation.tasks.reject')}
        </Button>
        <Button type="button" size="sm" variant="primary" disabled={busy}
          onClick={() => answer(task.jobId, prompt.requestId, 'once')}>
          {t('agentDelegation.tasks.allowOnce')}
        </Button>
      </div>
    </ConfigPageRow>
  ));

  /** Tasks still running or waiting, then the few that finished last. */
  const shownTasks = (assistant: LocalAssistant): DelegatedTask[] => {
    const own = (state?.tasks ?? []).filter(task => task.agentType === assistant.agentType);
    const active = own.filter(task => !FINISHED_TASK_STATES.has(task.state) || task.pendingPermissionCount > 0);
    const finished = own.filter(task => !active.includes(task)).slice(0, RECENT_FINISHED_TASKS);
    return [...active, ...finished];
  };

  const renderTasks = (assistant: LocalAssistant) => shownTasks(assistant).map(task => {
    const waitingCount = statuses[task.jobId] ? openPrompts(task).length : task.pendingPermissionCount;
    const waiting = waitingCount > 0
      ? t('agentDelegation.tasks.waiting', { count: String(waitingCount) })
      : null;
    return (
      <React.Fragment key={task.jobId}>
        <ConfigPageRow
          label={task.title || t('agentDelegation.tasks.untitled')}
          description={[describeSource(task), describeState(task.state), waiting].filter(Boolean).join(' · ')}
          align="center">
          <span />
        </ConfigPageRow>
        {renderPrompts(task, assistant.rootPath)}
      </React.Fragment>
    );
  });

  const renderOpenForm = (assistant: LocalAssistant) => {
    if (!state || draft?.agentType !== assistant.agentType) return null;
    const canSave = (draft.allDevices || draft.devices.length > 0 || state.connections.length > 0)
      && draft.usageConfirmed;
    return <>
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
                onChange={event => setDraft(current => current && ({
                  ...current,
                  devices: event.target.checked
                    ? [...current.devices.filter(item => item !== device.device_id), device.device_id]
                    : current.devices.filter(item => item !== device.device_id),
                }))} />
            ))}
          </div>
        </ConfigPageRow>
      )}
      <ConfigPageRow label={t('agentDelegation.usageNoticeTitle')} description={t('agentDelegation.usageNotice')}>
        <Checkbox checked={draft.usageConfirmed} disabled={busy}
          label={t('agentDelegation.usageConfirm')}
          onChange={event => setDraft({ ...draft, usageConfirmed: event.target.checked })} />
      </ConfigPageRow>
      {draftError && <div role="alert">{draftError}</div>}
      <ConfigPageRow label="" align="center" wide>
        <div className="openbitfun-runtime-settings__status-actions">
          <Button type="button" size="sm" variant="outline" disabled={busy}
            onClick={() => { setDraft(null); setDraftError(null); }}>
            {t('agentDelegation.cancel')}
          </Button>
          <Button type="button" size="sm" variant="primary" disabled={busy || !canSave} onClick={() => void saveDraft()}>
            {t('agentDelegation.save')}
          </Button>
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

  const renderAssistant = (assistant: LocalAssistant) => {
    const agent = state?.records
      .map(record => record.agent)
      .find((item): item is OpenAgent => item?.agentType === assistant.agentType);
    const granted = agent?.connections?.map(item => item.connectionId) ?? [];
    const description = !agent
      ? t('agentDelegation.notOpened')
      : agent.sameAccountDevices.length === 0
        ? t('agentDelegation.openedToAccounts')
        : t('agentDelegation.openedTo', { devices: describeDevices(agent) });
    return (
      <React.Fragment key={assistant.agentType}>
        <ConfigPageRow balanced label={assistantName(assistant)} description={description} align="center">
          <div className="openbitfun-runtime-settings__status-actions">
            {agent && grantDraft?.agentId !== agent.id && (
              <Button type="button" size="sm" variant="outline" disabled={busy}
                onClick={() => setGrantDraft({
                  agentId: agent.id,
                  connectionId: grantCandidates(granted)[0]?.connectionId ?? '',
                  ...NO_LIMITS,
                })}>
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
                  allDevices: false,
                  devices: [],
                  usageConfirmed: false,
                })}>
                {t('agentDelegation.open')}
              </Button>
            )}
          </div>
        </ConfigPageRow>
        {agent && closing === agent.id && renderCloseConfirm(agent, assistantName(assistant))}
        {renderOpenForm(assistant)}
        {agent && renderGrants(agent)}
        {agent && renderGrantPanel(agent.id, granted)}
        {renderTasks(assistant)}
      </React.Fragment>
    );
  };

  /** Entries an earlier build opened that are not assistants, or cannot be read. */
  const renderUnsupported = (record: OpenAgentRecord, index: number) => {
    const id = openAgentRecordId(record);
    return (
      <ConfigPageRow key={id ?? `unreadable-${index}`} balanced
        label={record.agent?.displayName || id || t('agentDelegation.unreadableName')}
        description={record.agent
          ? t('agentDelegation.unsupportedEntry')
          : t('agentDelegation.unreadable', { problem: record.problem ?? '' })}
        align="center">
        {id && (
          <Button type="button" size="sm" variant="outline" disabled={busy}
            onClick={() => void run(async () => { await delegationAPI.closeAgent(id); },
              'Failed to delete unsupported open-agent entry', t('agentDelegation.closeFailed'))}>
            {t('agentDelegation.delete')}
          </Button>
        )}
      </ConfigPageRow>
    );
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
    const unsupported = state.records.filter(record => !record.agent || !isAssistantAgentType(record.agent.agentType));
    return <>
      {state.assistants.length === 0 && (
        <ConfigPageRow label={t('agentDelegation.noAssistants')} align="center"><span /></ConfigPageRow>
      )}
      {state.assistants.map(renderAssistant)}
      {unsupported.map(renderUnsupported)}
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
