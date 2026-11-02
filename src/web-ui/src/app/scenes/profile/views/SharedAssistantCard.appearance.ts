import type { AppearanceSurfaceDescriptor } from '@/infrastructure/appearance';

export const sharedAssistantCardAppearanceDescriptor: AppearanceSurfaceDescriptor = {
  id: 'shared-assistant-card',
  parts: [
    { id: 'root' }, { id: 'main' }, { id: 'avatar' }, { id: 'title' },
    { id: 'badge' }, { id: 'metadata' }, { id: 'actions' }, { id: 'meta' },
  ],
};
