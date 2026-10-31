import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Button } from '@openbitfun/ui';
import {
  delegationAPI,
  type DelegationAuditEntry,
  type DelegationAuditRecord,
} from '@/infrastructure/api/service-api/DelegationAPI';
import { useI18n } from '@/infrastructure/i18n/hooks/useI18n';
import { usePeerDeviceModeOptional } from '@/infrastructure/peer-device/peerDeviceContextState';
import { isTauriRuntime } from '@/infrastructure/runtime';
import { createLogger } from '@/shared/utils/logger';
import {
  ConfigLoadingState,
  ConfigPageRow,
  ConfigPageSection,
  ConfigRetryState,
} from './common';

const log = createLogger('DelegationAuditView');

/** The newest entries read; older ones stay in the log. */
const AUDIT_LIMIT = 100;

/** Entries shown until the owner asks for all of them. */
const COLLAPSED_ENTRIES = 10;

/**
 * Tasks other accounts delegated to this device. The log is device-local, so
 * it is shown only on the device that keeps it; unreadable lines are listed
 * as such rather than hidden.
 */
export const DelegationAuditView: React.FC = () => {
  const { t, formatDate } = useI18n('settings/runtime');
  const peerMode = usePeerDeviceModeOptional()?.peerMode;
  const supported = isTauriRuntime() && peerMode?.active !== true;
  const [records, setRecords] = useState<DelegationAuditRecord[] | null>(null);
  const [loadError, setLoadError] = useState(false);
  /** Display names of the opened assistants, by open-agent id. */
  const [names, setNames] = useState<Record<string, string>>({});
  const [expanded, setExpanded] = useState(false);
  const activeRef = useRef(true);

  const load = useCallback(async () => {
    setLoadError(false);
    try {
      const [loaded, opened] = await Promise.all([
        delegationAPI.listAudit(AUDIT_LIMIT),
        // Names are optional: without them an entry names the assistant by its id.
        delegationAPI.listOpenAgents().catch(() => []),
      ]);
      if (!activeRef.current) return;
      setRecords(loaded);
      setNames(Object.fromEntries(opened
        .flatMap(record => (record.agent ? [[record.agent.id, record.agent.displayName] as const] : []))));
    } catch (error) {
      log.error('Failed to load the delegation audit log', { error });
      if (activeRef.current) setLoadError(true);
    }
  }, []);

  useEffect(() => {
    activeRef.current = true;
    if (supported) void load();
    return () => { activeRef.current = false; };
  }, [load, supported]);

  const describeEvent = (entry: DelegationAuditEntry): string => {
    switch (entry.event) {
      case 'submitted':
        return t('delegationAudit.submitted');
      case 'finished':
        return entry.state === 'succeeded'
          ? t('shared:statuses.done')
          : entry.state === 'cancelled'
            ? t('shared:statuses.cancelled')
            : t('delegationAudit.failed');
      default:
        return t('delegationAudit.otherEvent');
    }
  };

  const describeEntry = (entry: DelegationAuditEntry): string => {
    const at = Date.parse(entry.at);
    const summary = t('delegationAudit.entry', {
      time: Number.isNaN(at) ? entry.at : formatDate(at),
      agent: names[entry.openAgentId] || entry.openAgentId,
      name: entry.peerDisplayName || entry.connectionId,
      device: entry.peerDeviceName,
    });
    const trust = entry.trust === 'full'
      ? t('delegationAudit.trustFull')
      : entry.trust === undefined ? null : t('delegationAudit.trustNormal');
    const external = entry.externalAgents?.length
      ? t('delegationAudit.externalAgents', { agents: entry.externalAgents.join(', ') })
      : null;
    return [summary, trust, external, entry.detail].filter(Boolean).join(' · ');
  };

  const renderBody = () => {
    if (!supported) {
      return <ConfigPageRow label={t('delegationAudit.unsupported')} align="center"><span /></ConfigPageRow>;
    }
    if (loadError) {
      return <ConfigRetryState message={t('delegationAudit.loadFailed')} retryLabel={t('loading.retry')} onRetry={() => void load()} />;
    }
    if (!records) return <ConfigLoadingState label={t('delegationAudit.loading')} />;
    return <>
      {records.length === 0 && (
        <ConfigPageRow label={t('delegationAudit.empty')} align="center"><span /></ConfigPageRow>
      )}
      {[...records].reverse().slice(0, expanded ? undefined : COLLAPSED_ENTRIES).map((record, index) => (
        <ConfigPageRow
          key={`${record.entry?.jobId ?? 'unreadable'}-${record.entry?.event ?? ''}-${index}`}
          label={record.entry ? describeEvent(record.entry) : t('delegationAudit.unreadable')}
          description={record.entry ? describeEntry(record.entry) : record.problem ?? ''}
          align="center"
        >
          <span />
        </ConfigPageRow>
      ))}
      <ConfigPageRow label="" align="center">
        <div className="openbitfun-runtime-settings__status-actions">
          {!expanded && records.length > COLLAPSED_ENTRIES && (
            <Button type="button" size="sm" variant="outline" onClick={() => setExpanded(true)}>
              {t('delegationAudit.showAll', { count: String(records.length) })}
            </Button>
          )}
          <Button type="button" size="sm" variant="outline" onClick={() => void load()}>
            {t('delegationAudit.refresh')}
          </Button>
        </div>
      </ConfigPageRow>
    </>;
  };

  return (
    <ConfigPageSection
      title={t('delegationAudit.sectionTitle')}
      description={t('delegationAudit.sectionDescription')}
    >
      {renderBody()}
    </ConfigPageSection>
  );
};

export default DelegationAuditView;
