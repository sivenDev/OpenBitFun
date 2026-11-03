import type { AppearanceSurfaceDescriptor } from '@/infrastructure/appearance';

export const discoverPanelAppearanceDescriptor: AppearanceSurfaceDescriptor = {
  id: 'discover-panel',
  parts: [
    { id: 'problem' }, { id: 'devices' }, { id: 'device' }, { id: 'deviceHead' },
    { id: 'deviceName' }, { id: 'meta' }, { id: 'assistants' }, { id: 'assistant' },
    { id: 'avatar' }, { id: 'assistantName' }, { id: 'detail' }, { id: 'nameEdit' },
    { id: 'added' }, { id: 'otherSubnet' },
  ],
};
