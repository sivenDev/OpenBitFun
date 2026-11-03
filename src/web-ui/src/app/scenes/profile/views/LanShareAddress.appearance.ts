import type { AppearanceSurfaceDescriptor } from '@/infrastructure/appearance';

export const lanShareAddressAppearanceDescriptor: AppearanceSurfaceDescriptor = {
  id: 'lan-share-address',
  parts: [
    { id: 'thisDevice' }, { id: 'addresses' }, { id: 'address' }, { id: 'meta' },
    { id: 'form' }, { id: 'row' }, { id: 'found' }, { id: 'actions' },
  ],
};
