// @vitest-environment jsdom

import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import AssistantModelSection from './AssistantModelSection';

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

const sol = { id: 'sol', model_name: 'gpt-6.1-sol', name: 'Sol', enabled: true };
const terra = { id: 'terra', model_name: 'gpt-6.1-terra', name: 'Terra', enabled: true };
const luna = { id: 'luna', model_name: 'gpt-6.1-luna', name: 'Luna', enabled: false };

const mocks = vi.hoisted(() => ({
  assistantModel: null as unknown,
  save: vi.fn(async () => {}),
  notifyError: vi.fn(),
}));

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, options?: Record<string, unknown>) => (
      options?.name ? `${key}:${String(options.name)}` : key
    ),
  }),
}));
vi.mock('@openbitfun/ui', () => ({
  PageHeader: ({ title }: { title: string }) => <h3>{title}</h3>,
  Combobox: ({ value, options, onValueChange, 'aria-label': label }: {
    value: string;
    options: Array<{ value: string; label: string }>;
    onValueChange: (value: string) => void;
    'aria-label': string;
  }) => (
    <select aria-label={label} value={value} onChange={event => onValueChange(event.target.value)}>
      {options.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}
    </select>
  ),
}));
vi.mock('@/infrastructure/config/hooks/useAssistantModel', () => ({
  useAssistantModel: () => mocks.assistantModel,
}));
vi.mock('@/infrastructure/config/utils/assistantModel', () => ({
  saveAssistantDefaultModel: mocks.save,
}));
vi.mock('@/infrastructure/config/components/ModelSelectPresentation', () => ({
  useModelSelectPresentation: () => ({
    buildModelOption: (model: { id: string; model_name: string }) => ({ value: model.id, label: model.model_name }),
  }),
}));
vi.mock('@/infrastructure/config/services/modelCategory', () => ({
  filterSelectableTextChatModels: (models: Array<{ enabled: boolean }>) => models.filter(model => model.enabled),
}));
vi.mock('@/shared/notification-system', () => ({ notificationService: { error: mocks.notifyError } }));

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.clearAllMocks();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

const assistantModel = (overrides: Record<string, unknown>) => ({
  model: sol,
  source: 'primary',
  chosenId: null,
  supported: true,
  models: [sol, terra, luna],
  primaryModel: sol,
  ...overrides,
});

const select = () => container.querySelector('select')!;
const choose = async (value: string) => {
  await act(async () => {
    select().value = value;
    select().dispatchEvent(new Event('change', { bubbles: true }));
  });
};

it('follows the primary model by default and saves or clears a choice', async () => {
  mocks.assistantModel = assistantModel({});
  await act(async () => root.render(<AssistantModelSection assistantId="xiaolan" />));
  const labels = [...select().options].map(option => option.textContent);
  expect(labels).toEqual([
    'nursery.assistant.model.followPrimary:gpt-6.1-sol',
    'gpt-6.1-sol',
    'gpt-6.1-terra',
  ]);
  expect(container.querySelector('[data-openbitfun-part="modelNotice"]')).toBeNull();

  await choose('terra');
  expect(mocks.save).toHaveBeenLastCalledWith('xiaolan', 'terra');

  mocks.assistantModel = assistantModel({ model: terra, source: 'assistant', chosenId: 'terra' });
  await act(async () => root.render(<AssistantModelSection assistantId="xiaolan" />));
  expect(select().value).toBe('terra');
  await choose(select().options[0].value);
  expect(mocks.save).toHaveBeenLastCalledWith('xiaolan', null);
});

it('keeps an unavailable choice listed and says which model runs meanwhile', async () => {
  mocks.assistantModel = assistantModel({ source: 'chosenUnavailable', chosenId: 'luna' });
  await act(async () => root.render(<AssistantModelSection assistantId="xiaolan" />));
  expect(select().value).toBe('luna');
  expect(select().options[select().selectedIndex].textContent)
    .toBe('nursery.assistant.model.unavailableOption:gpt-6.1-luna');
  expect(container.querySelector('[data-openbitfun-part="modelNotice"]')?.textContent)
    .toBe('nursery.assistant.model.unavailableNotice:gpt-6.1-sol');
});

it('reports a failed save and hides on hosts without the setting', async () => {
  mocks.assistantModel = assistantModel({});
  mocks.save.mockRejectedValueOnce(new Error('offline'));
  await act(async () => root.render(<AssistantModelSection assistantId="xiaolan" />));
  await choose('terra');
  expect(mocks.notifyError).toHaveBeenCalledWith('nursery.assistant.model.saveFailed');

  mocks.assistantModel = assistantModel({ supported: false });
  await act(async () => root.render(<AssistantModelSection assistantId="xiaolan" />));
  expect(container.querySelector('select')).toBeNull();
});
