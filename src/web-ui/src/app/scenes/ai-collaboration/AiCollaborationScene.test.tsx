// @vitest-environment jsdom
import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import AiCollaborationScene from './AiCollaborationScene';

vi.mock('@/infrastructure/i18n/hooks/useI18n', () => ({
  useI18n: () => ({ t: (key: string) => key }),
}));
vi.mock('@/infrastructure/config/components/common', () => ({
  ConfigPageLayout: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  ConfigPageHeader: ({ title, subtitle }: { title: string; subtitle?: string }) => <h1>{title}|{subtitle}</h1>,
  ConfigPageContent: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
}));
vi.mock('@/infrastructure/config/components/AgentDelegationConfig', () => ({
  AgentDelegationConfig: () => <section>agent-delegation</section>,
}));
vi.mock('@/infrastructure/config/components/ConnectionsConfig', () => ({
  ConnectionsConfig: () => <section>connections</section>,
}));

describe('AiCollaborationScene', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  it('holds the opened assistants and the connections', async () => {
    await act(async () => { root.render(<AiCollaborationScene />); });
    expect(container.querySelector('[data-openbitfun-scene="ai-collaboration"]')).not.toBeNull();
    expect(container.textContent).toContain('aiCollaboration.title|aiCollaboration.subtitle');
    const sections = [...container.querySelectorAll('section')].map(section => section.textContent);
    expect(sections).toEqual(['agent-delegation', 'connections']);
  });
});
