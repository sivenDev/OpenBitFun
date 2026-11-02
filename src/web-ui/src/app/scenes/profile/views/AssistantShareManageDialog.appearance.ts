import type { AppearanceSurfaceDescriptor } from '@/infrastructure/appearance';

export const assistantShareManageAppearanceDescriptor: AppearanceSurfaceDescriptor = {
  id: 'assistant-share-manage',
  parts: [
    { id: 'summary' }, { id: 'section' }, { id: 'heading' }, { id: 'empty' },
    { id: 'list' }, { id: 'row' }, { id: 'who' }, { id: 'meta' },
  ],
};
