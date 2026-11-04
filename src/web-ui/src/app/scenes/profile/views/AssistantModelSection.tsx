import React, { useCallback, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { Combobox, PageHeader } from '@openbitfun/ui';
import { useAssistantModel } from '@/infrastructure/config/hooks/useAssistantModel';
import { useModelSelectPresentation } from '@/infrastructure/config/components/ModelSelectPresentation';
import { filterSelectableTextChatModels } from '@/infrastructure/config/services/modelCategory';
import { saveAssistantDefaultModel } from '@/infrastructure/config/utils/assistantModel';
import type { AIModelConfig } from '@/infrastructure/config/types';
import { notificationService } from '@/shared/notification-system';
import { createLogger } from '@/shared/utils/logger';

const log = createLogger('AssistantModelSection');

/** The option that clears the assistant's own choice. */
const FOLLOW_PRIMARY = '__primary__';

const modelName = (model: AIModelConfig | null | undefined) =>
  model ? model.model_name || model.name || model.id || '' : '';

interface AssistantModelSectionProps {
  /** The assistant's directory name, as in `assistant:<id>`. */
  assistantId: string;
}

/** Chooses the model this assistant runs on, for its own chats and for calls. */
const AssistantModelSection: React.FC<AssistantModelSectionProps> = ({ assistantId }) => {
  const { t } = useTranslation('scenes/profile');
  const { buildModelOption } = useModelSelectPresentation();
  const assistantModel = useAssistantModel(assistantId);

  const options = useMemo(() => {
    if (!assistantModel) return [];
    const primaryName = modelName(assistantModel.primaryModel);
    const choices = [
      {
        value: FOLLOW_PRIMARY,
        label: primaryName
          ? t('nursery.assistant.model.followPrimary', { name: primaryName })
          : t('nursery.assistant.model.followPrimaryUnset'),
      },
      ...filterSelectableTextChatModels(assistantModel.models)
        .filter(model => model.id)
        .map(buildModelOption),
    ];
    const { chosenId } = assistantModel;
    if (chosenId && !choices.some(option => option.value === chosenId)) {
      const missing = assistantModel.models.find(model => model.id === chosenId);
      choices.push({
        value: chosenId,
        label: t('nursery.assistant.model.unavailableOption', { name: modelName(missing) || chosenId }),
      });
    }
    return choices;
  }, [assistantModel, buildModelOption, t]);

  const handleChange = useCallback((value: string | number | (string | number)[]) => {
    const selected = String(Array.isArray(value) ? value[0] ?? '' : value);
    const modelId = !selected || selected === FOLLOW_PRIMARY ? null : selected;
    void saveAssistantDefaultModel(assistantId, modelId).catch(error => {
      log.error('Failed to save the assistant default model', { assistantId, error });
      notificationService.error(t('nursery.assistant.model.saveFailed'));
    });
  }, [assistantId, t]);

  if (!assistantModel?.supported) return null;
  const { chosenId, source } = assistantModel;
  const fallbackName = modelName(assistantModel.model);
  return (
    <section
      className="acp-section"
      aria-label={t('nursery.assistant.model.title')}
      data-openbitfun-component="assistant-config-page"
      data-openbitfun-part="model"
    >
      <PageHeader size="sm" level={3} title={t('nursery.assistant.model.title')} />
      <Combobox
        aria-label={t('nursery.assistant.model.title')}
        value={chosenId ?? FOLLOW_PRIMARY}
        onValueChange={handleChange}
        options={options}
        size="sm"
      />
      {source === 'chosenUnavailable' ? (
        <p
          className="acp-empty"
          role="status"
          data-openbitfun-component="assistant-config-page"
          data-openbitfun-part="modelNotice"
        >
          {fallbackName
            ? t('nursery.assistant.model.unavailableNotice', { name: fallbackName })
            : t('nursery.assistant.model.unavailableNoticeUnset')}
        </p>
      ) : null}
    </section>
  );
};

export default AssistantModelSection;
