import React, {
  useCallback,
  useSyncExternalStore,
} from 'react';
import { useI18n } from '@/infrastructure/i18n/hooks/useI18n';

import { flowChatStore } from '../store/FlowChatStore';
import {
  resolveSubagentAvatarAccent,
  resolveSubagentNameKey,
  SubagentAvatar,
  SubagentDelegationAvatar,
} from '../subagent-identity';
import type { FlowToolItem, ToolCardProps } from '../types/flow-chat';
import {
  sessionLineageLifecycleForSession,
  type SessionLineageLifecycle,
} from '../utils/sessionLineage';
import { openBtwSessionInAuxPane } from '../services/btwSessionPane';
import { AgentControlToolCard as AgentControlToolCardView } from '@openbitfun/ui/flow-chat';
import { AgentInteractionToolCard } from './AgentInteractionToolCard';
import { readToolRecord, resolveInteractionAgentSessionId, toolString } from './toolInteractionModel';
import { recordedBackgroundOutcome } from './agentWaitAvatarTargets';
import {
  calledAssistantName,
  calledAssistantOwner,
  calledTypeInConversation,
  sharedConversationOwnerDevice,
  calledAssistantSessionName,
  latestEffectiveModel,
  reportedSharedModel,
  localAssistantFromAgentType,
  remoteAgentFromAgentType,
} from '../utils/remoteAgent';
import { workspaceManager } from '@/infrastructure/services/business/workspaceManager';
import { useLanShareStore } from '@/infrastructure/lan-share/lanShareStore';
import { useAssistantModel } from '@/infrastructure/config/hooks/useAssistantModel';
import { plannedAssistantModelLabel } from './plannedAssistantModel';

const PARAMETER_STREAMING_STATUSES = new Set<FlowToolItem['status']>([
  'preparing',
  'streaming',
  'receiving',
]);

function readString(source: unknown, ...keys: string[]): string {
  if (!source || typeof source !== 'object') {
    return '';
  }

  const record = source as Record<string, unknown>;
  for (const key of keys) {
    const value = record[key];
    if (typeof value === 'string' && value.trim()) {
      return value.trim();
    }
  }
  return '';
}

function fallbackLifecycle(status: FlowToolItem['status']): SessionLineageLifecycle {
  switch (status) {
    case 'completed':
    case 'confirmed':
      return 'completed';
    case 'cancelled':
    case 'rejected':
      return 'cancelled';
    case 'error':
      return 'error';
    case 'waiting':
      return 'waiting';
    case 'pending':
    case 'queued':
      return 'idle';
    default:
      return 'running';
  }
}

function lifecycleForOutcome(outcome: string | null): SessionLineageLifecycle | null {
  switch (outcome) {
    case 'completed':
      return 'completed';
    case 'failed':
    case 'partial_timeout':
      return 'error';
    case 'cancelled':
    case 'interrupted':
      return 'cancelled';
    default:
      return null;
  }
}

function statusForLifecycle(
  lifecycle: SessionLineageLifecycle,
  fallback: FlowToolItem['status'],
): FlowToolItem['status'] {
  switch (lifecycle) {
    case 'running':
    case 'finishing':
      return 'running';
    case 'waiting':
      return 'waiting';
    case 'completed':
      return 'completed';
    case 'error':
      return 'error';
    case 'cancelled':
      return 'cancelled';
    default:
      return fallback;
  }
}

function subscribeToFlowChatStore(listener: () => void): () => void {
  return flowChatStore.subscribe(() => listener());
}

function readLinkedAgentSnapshot(sessionId: string): string {
  if (!sessionId) {
    return '';
  }

  const session = flowChatStore.getState().sessions.get(sessionId);
  const latestTurn = session?.dialogTurns?.[session.dialogTurns.length - 1];
  return JSON.stringify([
    session?.title ?? '',
    session?.mode ?? '',
    session?.subagentType ?? '',
    session?.config?.agentType ?? '',
    session?.config?.modelName ?? '',
    session?.needsUserAttention ?? false,
    session?.status ?? '',
    session?.persistedStatus ?? '',
    session?.hasUnreadCompletion ?? '',
    latestTurn?.id ?? '',
    latestTurn?.status ?? '',
    latestTurn?.modelRounds?.some(round => round.isStreaming) ?? false,
  ]);
}

const AgentSpawnCard: React.FC<ToolCardProps> = ({
  toolItem,
  sessionId,
}) => {
  const { t, currentLanguage } = useI18n('flow-chat');
  const { toolCall, status } = toolItem;
  const toolId = toolItem.id ?? toolCall?.id;
  const params = toolItem.partialParams ?? toolCall?.input;
  // In a conversation with a shared assistant, `assistant:<id>` is one of
  // its owner's assistants.
  const conversationOwner = useSyncExternalStore(
    subscribeToFlowChatStore,
    () => sharedConversationOwnerDevice(sessionId ? flowChatStore.getState().sessions.get(sessionId) : null),
  );
  const localDeviceId = useLanShareStore(state => state.overview?.status.deviceId);
  const inputAgentType = calledTypeInConversation(
    readString(params, 'agent_type', 'agentType'),
    conversationOwner,
    localDeviceId,
  );
  const readLinkedSession = useCallback(() => {
    const { sessions } = flowChatStore.getState();
    return resolveInteractionAgentSessionId(toolItem, sessionId ? sessions.get(sessionId) : undefined,
      { sessions, parentSessionId: sessionId });
  }, [toolItem, sessionId]);
  const linkedSubagentSessionId = useSyncExternalStore(
    subscribeToFlowChatStore, readLinkedSession, readLinkedSession,
  );
  const readSnapshot = useCallback(
    () => readLinkedAgentSnapshot(linkedSubagentSessionId),
    [linkedSubagentSessionId],
  );

  useSyncExternalStore(
    subscribeToFlowChatStore,
    readSnapshot,
    readSnapshot,
  );

  const linkedSession = linkedSubagentSessionId
    ? flowChatStore.getState().sessions.get(linkedSubagentSessionId)
    : undefined;
  // A started spawn reads as completed; until the called session loads, a
  // later wait in this session is what says how the work actually ended.
  const readRecordedOutcome = useCallback(() => sessionId
    ? recordedBackgroundOutcome(flowChatStore.getState().sessions.get(sessionId),
      toolString(readToolRecord(toolItem.toolResult?.result).bg_task_id))
    : null, [sessionId, toolItem.toolResult?.result]);
  const recordedOutcome = useSyncExternalStore(
    subscribeToFlowChatStore, readRecordedOutcome, readRecordedOutcome,
  );
  const lifecycle = linkedSession
    ? sessionLineageLifecycleForSession(linkedSession)
    : lifecycleForOutcome(recordedOutcome) ?? fallbackLifecycle(status);
  const stableAgentType = linkedSession?.mode?.trim()
    || linkedSession?.config?.agentType?.trim()
    || inputAgentType;
  const stableSubagentType = linkedSession?.subagentType?.trim() || inputAgentType;
  // An assistant answers under its own name and model, not a subagent's.
  const sharedWithMe = useLanShareStore(state => state.sharedWithMe);
  // The requested type names the assistant; the linked session runs as
  // `Claw` and its own type does not.
  const requestsAssistant = (type: string | undefined) =>
    Boolean(localAssistantFromAgentType(type) || remoteAgentFromAgentType(type));
  const calledType = requestsAssistant(inputAgentType)
    ? inputAgentType
    : stableSubagentType || stableAgentType;
  const assistantWorkspaces = workspaceManager.getState().assistantWorkspaces;
  const assistantName = calledAssistantName(calledType, assistantWorkspaces, sharedWithMe)
    ?? calledAssistantSessionName(linkedSession, assistantWorkspaces);
  const callsAssistant = requestsAssistant(calledType) || Boolean(linkedSession?.delegatedBy);
  // Name whose assistant it is, never its internal type.
  const assistantOwner = calledAssistantOwner(calledType, linkedSession, sharedWithMe);
  const assistantOwnerLabel = assistantOwner?.kind === 'shared'
    ? t('toolCards.taskTool.assistantOwnerShared', { owner: assistantOwner.owner })
    : assistantOwner ? t('toolCards.taskTool.assistantOwnerMine') : null;
  const agentDisplayName = assistantName
    ?? (callsAssistant ? t('toolCards.taskTool.unnamedAssistant') : null)
    ?? (linkedSubagentSessionId
      ? t(resolveSubagentNameKey(linkedSubagentSessionId)) : t('toolCards.interaction.unknownAgent'));
  // A called assistant runs on the model its session actually used; before it
  // runs, a local one names the model its settings choose and a shared one
  // its owner's device's choice.
  const plannedModel = plannedAssistantModelLabel(
    useAssistantModel(localAssistantFromAgentType(calledType)),
    t,
  );
  const assistantRanOn = remoteAgentFromAgentType(calledType)
    ? reportedSharedModel(calledType, toolItem.subagentModelDisplayName, latestEffectiveModel(linkedSession))
    : toolItem.subagentModelDisplayName?.trim() || latestEffectiveModel(linkedSession);
  const modelName = callsAssistant ? assistantRanOn || (assistantOwner?.kind === 'shared'
    ? t('toolCards.taskTool.remoteAgentModel')
    : plannedModel || t('toolCards.taskTool.assistantModelDefault'))
    : toolItem.subagentModelDisplayName?.trim()
    || linkedSession?.config?.modelName?.trim()
    || toolItem.subagentModelId?.trim()
    || readString(params, 'model_id', 'modelId')
    || t('subagentIdentity.preview.unknownModel');
  const isParameterStreaming = Boolean(toolItem.isParamsStreaming)
    || PARAMETER_STREAMING_STATUSES.has(status);
  const displayStatus = isParameterStreaming
    ? status
    : statusForLifecycle(lifecycle, status);
  const canOpenSession = Boolean(linkedSubagentSessionId && sessionId);
  const isSpawn = toolItem.toolName === 'AgentSpawn';

  const handleOpenSession = useCallback((event: React.MouseEvent<HTMLButtonElement>) => {
    event.stopPropagation();
    if (!linkedSubagentSessionId || !sessionId) {
      return;
    }

    const parentSession = flowChatStore.getState().sessions.get(sessionId);
    openBtwSessionInAuxPane({
      childSessionId: linkedSubagentSessionId,
      parentSessionId: sessionId,
      // A called assistant's session lives in that assistant's workspace.
      workspaceId: linkedSession?.workspaceId,
      workspacePath: parentSession?.workspacePath,
      sessionKind: 'subagent',
      sessionTitle: agentDisplayName,
      agentType: stableAgentType || undefined,
      parentToolCallId: toolCall?.id || toolItem.id,
      subagentType: stableSubagentType || undefined,
      remoteConnectionId: parentSession?.remoteConnectionId,
      remoteSshHost: parentSession?.remoteSshHost,
      includeInternal: true,
    });
  }, [
    agentDisplayName,
    linkedSession?.workspaceId,
    linkedSubagentSessionId,
    sessionId,
    stableAgentType,
    stableSubagentType,
    toolCall?.id,
    toolItem.id,
  ]);

  const statusTone = lifecycle === 'running' || lifecycle === 'finishing'
    ? 'success'
    : lifecycle === 'waiting'
      ? 'warning'
      : lifecycle === 'error' || lifecycle === 'cancelled'
        ? 'danger'
        : 'neutral';

  return (
    <div
      data-openbitfun-adapter="agent-control-tool-card"
      data-tool-card-id={toolId ?? ''}
    >
      <AgentControlToolCardView
        lang={currentLanguage}
        status={displayStatus}
        agentName={agentDisplayName}
        action={toolItem.toolName === 'AgentSendInput' ? t('toolCards.agentSendInput.action') : undefined}
        accentColor={linkedSubagentSessionId ? resolveSubagentAvatarAccent(linkedSubagentSessionId) : undefined}
        summary={readString(params, 'description', 'title', 'prompt', 'message', 'input') || t('subagentIdentity.preview.noDescription')}
        avatar={isSpawn ? (
          <SubagentDelegationAvatar
            sessionId={linkedSubagentSessionId}
            pending={['pending', 'queued', 'preparing', 'streaming', 'receiving', 'running'].includes(status)}
            name={agentDisplayName} size={40} showStatus={false} status={lifecycle}
          />
        ) : linkedSubagentSessionId ? (
          <SubagentAvatar
            sessionId={linkedSubagentSessionId}
            name={agentDisplayName}
            size={40}
            motion
            showStatus={false}
            status={lifecycle}
          />
        ) : undefined}
        preview={{
          avatar: linkedSubagentSessionId ? (
            <SubagentAvatar sessionId={linkedSubagentSessionId} name={agentDisplayName}
              size={40} showStatus={false} status={lifecycle} />
          ) : isSpawn ? <SubagentDelegationAvatar pending={false} motion={false} size={40} showStatus={false} /> : undefined,
          agentType: assistantOwnerLabel
            ?? (stableSubagentType || stableAgentType || t('toolCards.taskTool.defaultAgentKind')),
          model: modelName,
          labels: {
            agentType: t('subagentIdentity.preview.agentType'),
            model: t('subagentIdentity.preview.model'),
            description: t('subagentIdentity.preview.description'),
          },
        }}
        statusLabel={lifecycle === 'completed' ? undefined
          : t(`flowChatHeader.agentTreeStatus.${lifecycle === 'finishing' ? 'running' : lifecycle}`)}
        statusTone={statusTone}
        onOpenAgent={canOpenSession ? handleOpenSession : undefined}
        openAgentLabel={t('toolCards.taskTool.openInPanel')}

      />
    </div>
  );
};

export const AgentControlToolCard: React.FC<ToolCardProps> = props => props.toolItem.toolName === 'AgentSpawn'
  ? <AgentSpawnCard {...props} /> : <AgentInteractionToolCard {...props} />;
