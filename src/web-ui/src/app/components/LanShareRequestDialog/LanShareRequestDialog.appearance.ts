import type { AppearanceSurfaceDescriptor } from '@/infrastructure/appearance';

export const lanShareRequestAppearanceDescriptor: AppearanceSurfaceDescriptor = {
  id: 'lan-share-request',
  parts: [
    { id: 'message' }, { id: 'code' }, { id: 'hint' }, { id: 'actions' },
  ],
};
