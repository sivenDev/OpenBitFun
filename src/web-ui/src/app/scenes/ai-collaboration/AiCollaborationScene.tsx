import React from 'react';
import '@/app/scenes/settings/pages/shared/RuntimeSettings.scss';
import {
  ConfigPageContent,
  ConfigPageHeader,
  ConfigPageLayout,
} from '@/infrastructure/config/components/common';
import { AgentDelegationConfig } from '@/infrastructure/config/components/AgentDelegationConfig';
import { ConnectionsConfig } from '@/infrastructure/config/components/ConnectionsConfig';
import { DelegationAuditView } from '@/infrastructure/config/components/DelegationAuditView';
import { useI18n } from '@/infrastructure/i18n/hooks/useI18n';
import './AiCollaborationScene.scss';

/** Agent collaboration: what this device opens to others, its connections to
 *  other accounts, and the tasks other accounts sent here. */
const AiCollaborationScene: React.FC = () => {
  const { t } = useI18n('common');
  return (
    <div
      className="openbitfun-ai-collaboration"
      data-openbitfun-scene="ai-collaboration"
      data-openbitfun-part="root"
    >
      <ConfigPageLayout className="openbitfun-runtime-settings">
        <ConfigPageHeader
          title={t('aiCollaboration.title')}
          subtitle={t('aiCollaboration.subtitle')}
        />
        <ConfigPageContent>
          <AgentDelegationConfig />
          <ConnectionsConfig />
          <DelegationAuditView />
        </ConfigPageContent>
      </ConfigPageLayout>
    </div>
  );
};

export default AiCollaborationScene;
