import React, { useCallback, useMemo, useState, useSyncExternalStore } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, OverflowText, PageHeader } from '@openbitfun/ui';
import { flowChatStore } from '@/flow_chat/store/FlowChatStore';
import { openMainSession } from '@/flow_chat/services/sessionActivation';
import { calledSessionsIn } from './calledSessions';
import { i18nService } from '@/infrastructure/i18n';
import { createLogger } from '@/shared/utils/logger';

const log = createLogger('CalledByAssistantsSection');

interface CalledByAssistantsSectionProps {
  workspaceId?: string;
}

const subscribe = (listener: () => void) => flowChatStore.subscribe(() => listener());
const readSessions = () => flowChatStore.getState().sessions;

/**
 * Work this assistant did for other assistants. Those calls are shown at the
 * caller; this folded record keeps them findable here without crowding the
 * assistant's own conversation list.
 */
const CalledByAssistantsSection: React.FC<CalledByAssistantsSectionProps> = ({ workspaceId }) => {
  const { t } = useTranslation('scenes/profile');
  const [expanded, setExpanded] = useState(false);
  const sessions = useSyncExternalStore(subscribe, readSessions, readSessions);
  const calls = useMemo(() => calledSessionsIn(sessions.values(), workspaceId), [sessions, workspaceId]);

  const open = useCallback((sessionId: string) => {
    void openMainSession(sessionId, { workspaceId }).catch(error => {
      log.error('Failed to open a session another assistant started', error);
    });
  }, [workspaceId]);

  if (calls.length === 0) return null;
  return (
    <section
      className="acp-section"
      aria-label={t('nursery.assistant.calledBy.title')}
      data-openbitfun-component="assistant-config-page"
      data-openbitfun-part="calledBy"
    >
      <PageHeader size="sm" level={3} title={t('nursery.assistant.calledBy.title')} />
      <Button
        variant="text"
        aria-expanded={expanded}
        onClick={() => setExpanded(value => !value)}
      >
        {expanded
          ? t('nursery.assistant.calledBy.hide')
          : t('nursery.assistant.calledBy.show', { count: calls.length })}
      </Button>
      {expanded ? (
        <ul className="acp-called-by-list">
          {calls.map(session => {
            const caller = session.delegatedBy?.assistantName?.trim()
              || t('nursery.assistant.calledBy.unknownCaller');
            const when = session.lastActiveAt
              ? i18nService.formatDate(new Date(session.lastActiveAt), { dateStyle: 'short', timeStyle: 'short' })
              : '';
            return (
              <li key={session.sessionId}>
                <Button variant="text" onClick={() => open(session.sessionId)}>
                  <OverflowText>
                    {t('nursery.assistant.calledBy.row', {
                      caller,
                      title: session.title || t('nursery.assistant.calledBy.untitled'),
                      when,
                    })}
                  </OverflowText>
                </Button>
              </li>
            );
          })}
        </ul>
      ) : null}
    </section>
  );
};

export default CalledByAssistantsSection;
