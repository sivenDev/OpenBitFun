import { useI18n } from '@/infrastructure/i18n/hooks/useI18n';
import type { AddressProblem } from '@/infrastructure/api/service-api/LanShareAPI';

/** The sentence that says why an address cannot be used to add a device. */
export function useAddressProblemText(): (problem: AddressProblem) => string {
  const { t } = useI18n('scenes/profile');
  return (problem) => t(`nursery.discover.byAddress.problems.${problem}`);
}
