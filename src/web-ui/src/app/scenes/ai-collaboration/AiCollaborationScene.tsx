import React from 'react';
import '@/app/scenes/settings/pages/shared/RuntimeSettings.scss';
import {
  ConfigPageContent,
  ConfigPageHeader,
  ConfigPageLayout,
} from '@/infrastructure/config/components/common';
import { AgentDelegationConfig } from '@/infrastructure/config/components/AgentDelegationConfig';
import { ConnectionsConfig } from '@/infrastructure/config/components/ConnectionsConfig';
import { useI18n } from '@/infrastructure/i18n/hooks/useI18n';
import './AiCollaborationScene.scss';

/** Agent collaboration: the assistants this device opens to others, with the
 *  tasks they run for them, and its connections to other accounts. */
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
        </ConfigPageContent>
      </ConfigPageLayout>
    </div>
  );
};

export default AiCollaborationScene;
