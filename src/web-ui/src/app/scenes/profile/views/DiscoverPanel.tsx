import React, { useEffect, useRef, useState } from 'react';
import {
  Alert,
  Button,
  Dialog,
  DialogBody,
  DialogClose,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogHeading,
  DialogTitle,
  Empty,
  Input,
  Spinner,
  Tooltip,
} from '@openbitfun/ui';
import { AssistantAvatar } from '@/app/components/AssistantAvatar';
import { confirmDanger } from '@/infrastructure/confirm-dialog';
import { useI18n } from '@/infrastructure/i18n/hooks/useI18n';
import {
  lanShareAPI,
  shownAddress,
  type DiscoveredAssistant,
  type DiscoveredDevice,
  type ShareAccess,
} from '@/infrastructure/api/service-api/LanShareAPI';
import { currentAccess, secondsLeft, useLanShareStore } from '@/infrastructure/lan-share/lanShareStore';
import { useNotification } from '@/shared/notification-system';
import { createLogger } from '@/shared/utils/logger';
import { AddByAddress, ThisDeviceAddress } from './LanShareAddress';
import { useAddressProblemText } from './useAddressProblemText';
import './DiscoverPanel.scss';

const log = createLogger('DiscoverPanel');
const TICK_MS = 500;
/** How long the panel looks before saying nobody shares anything. */
const SEARCH_MS = 4000;

interface DiscoverPanelProps {
  open: boolean;
  onClose: () => void;
  /** An owner allowed a request: the assistant is now in the person's list. */
  onGranted?: () => void;
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function accessKey(device: DiscoveredDevice, assistant: DiscoveredAssistant): string {
  return `${device.publicKey}/${assistant.id}`;
}

/**
 * Assistants other devices share on the local network, by device, like the
 * Finder's "Shared" list. Asking for one sends a request the owner answers
 * within a minute; the answer arrives here at once.
 */
const DiscoverPanel: React.FC<DiscoverPanelProps> = ({ open, onClose, onGranted }) => {
  const { t } = useI18n('scenes/profile');
  const notification = useNotification();
  const devices = useLanShareStore((state) => state.discovered);
  const status = useLanShareStore((state) => state.overview?.status);
  const [problem, setProblem] = useState<string | null>(null);
  const [searching, setSearching] = useState(true);
  const [asking, setAsking] = useState<string | null>(null);
  // Why the last request was not sent. Shown in the panel: a toast would sit
  // under this dialog until it closes.
  const [requestProblem, setRequestProblem] = useState<string | null>(null);
  const [editingName, setEditingName] = useState<string | null>(null);
  const [savingName, setSavingName] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  // A device added by address whose address is being changed.
  const [movingDevice, setMovingDevice] = useState<{ deviceId: string; address: string } | null>(null);
  const [moveProblem, setMoveProblem] = useState<string | null>(null);
  const [movingBusy, setMovingBusy] = useState(false);
  const problemText = useAddressProblemText();
  const previous = useRef(new Map<string, { access: ShareAccess['state']; name: string }>());

  useEffect(() => {
    if (!open) return undefined;
    let active = true;
    setProblem(null);
    setRequestProblem(null);
    setSearching(true);
    void lanShareAPI.discoverStart()
      .then((found) => {
        if (active) useLanShareStore.setState({ discovered: found });
      })
      .catch((error: unknown) => {
        log.warn('Discover did not start', { error: message(error) });
        if (active) setProblem(message(error));
      });
    const searched = window.setTimeout(() => setSearching(false), SEARCH_MS);
    return () => {
      active = false;
      window.clearTimeout(searched);
      void lanShareAPI.discoverStop().catch((error: unknown) => {
        log.debug('Discover did not stop', { error: message(error) });
      });
    };
  }, [open]);

  const counting = devices.some((device) => device.assistants.some(
    (assistant) => {
      const access = currentAccess(assistant.access, now);
      return access.state === 'pending' || access.state === 'cooling';
    },
  ));
  useEffect(() => {
    if (!open || !counting) return undefined;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), TICK_MS);
    return () => window.clearInterval(timer);
  }, [counting, open]);

  // Say how a request ended when its answer arrives.
  useEffect(() => {
    const current = new Map<string, { access: ShareAccess['state']; name: string }>();
    let granted = false;
    for (const device of devices) {
      for (const assistant of device.assistants) {
        const key = accessKey(device, assistant);
        current.set(key, { access: assistant.access.state, name: assistant.name });
        if (previous.current.get(key)?.access !== 'pending') continue;
        if (assistant.access.state === 'granted') {
          notification.success(t('nursery.discover.granted', { name: assistant.name }));
          granted = true;
        } else if (assistant.access.state !== 'pending') {
          notification.info(t('nursery.discover.declined', { name: assistant.name }));
        }
      }
    }
    previous.current = current;
    if (granted) onGranted?.();
  }, [devices, notification, onGranted, t]);

  const ask = async (device: DiscoveredDevice, assistant: DiscoveredAssistant) => {
    const key = accessKey(device, assistant);
    if (asking) return;
    setAsking(key);
    setRequestProblem(null);
    try {
      const opened = await lanShareAPI.request(device.deviceId, assistant.id);
      if (opened.granted === true) {
        notification.success(t('nursery.discover.granted', { name: assistant.name }));
        onGranted?.();
      } else if (opened.granted === false) {
        notification.info(t('nursery.discover.declined', { name: assistant.name }));
      }
    } catch (error) {
      log.warn('Share request not sent', { deviceId: device.deviceId, assistantId: assistant.id, error: message(error) });
      await useLanShareStore.getState().refreshDiscovered();
      // A device that went away says so under its name instead.
      const now = useLanShareStore.getState().discovered.find((found) => found.publicKey === device.publicKey);
      if (now?.status.state === 'ready') {
        setRequestProblem(t('nursery.discover.requestFailed', { reason: message(error) }));
      }
      return;
    } finally {
      setAsking(null);
    }
    void useLanShareStore.getState().refreshDiscovered();
  };

  const saveAddress = async () => {
    if (!movingDevice || !movingDevice.address.trim() || movingBusy) return;
    setMovingBusy(true);
    try {
      const outcome = await lanShareAPI.setDeviceAddress(movingDevice.deviceId, movingDevice.address);
      if (outcome.state === 'problem') {
        setMoveProblem(problemText(outcome.problem));
        return;
      }
      setMovingDevice(null);
      setMoveProblem(null);
      void useLanShareStore.getState().refreshDiscovered();
    } catch (error) {
      setMoveProblem(t('nursery.discover.byAddress.failed', { reason: message(error) }));
    } finally {
      setMovingBusy(false);
    }
  };

  const removeDevice = async (device: DiscoveredDevice) => {
    const confirmed = await confirmDanger(
      t('nursery.discover.removeDeviceTitle', { name: device.displayName }),
      t('nursery.discover.removeDeviceMessage'),
      { confirmText: t('nursery.discover.removeDevice') },
    );
    if (!confirmed) return;
    try {
      await lanShareAPI.removeDevice(device.deviceId);
      void useLanShareStore.getState().refreshDiscovered();
    } catch (error) {
      log.warn('Device added by address not removed', { deviceId: device.deviceId, error: message(error) });
      notification.error(t('nursery.discover.removeDeviceFailed', { reason: message(error) }));
    }
  };

  const saveName = async () => {
    const name = editingName?.trim();
    if (!name || savingName) return;
    setSavingName(true);
    try {
      await lanShareAPI.setDisplayName(name);
      setEditingName(null);
      void useLanShareStore.getState().refreshOverview();
    } catch (error) {
      notification.error(t('nursery.discover.nameFailed', { reason: message(error) }));
    } finally {
      setSavingName(false);
    }
  };

  const openSettings = () => {
    void lanShareAPI.openNetworkSettings().catch((error: unknown) => {
      notification.error(t('nursery.discover.settingsFailed', { reason: message(error) }));
    });
  };

  const settingsButton = (
    <Button size="sm" variant="fill" onClick={openSettings} data-testid="discover-open-settings">
      {t('nursery.discover.openSettings')}
    </Button>
  );

  const accessText = (access: ShareAccess): string => {
    switch (access.state) {
      case 'pending':
        return t('nursery.discover.pending', { seconds: secondsLeft(access.deadlineMs, now) });
      case 'granted':
        return t('nursery.discover.added');
      case 'cooling':
        return t('nursery.discover.cooling', { seconds: secondsLeft(access.retryAtMs, now) });
      default:
        return t('nursery.discover.ask');
    }
  };

  const deviceNote = (device: DiscoveredDevice): string | null => {
    switch (device.status.state) {
      case 'connecting':
        return t('nursery.discover.reading');
      case 'needsUpdate':
        return t('nursery.discover.needsUpdate');
      case 'unreachable':
        return t('nursery.discover.unreachable');
      case 'identityChanged':
        return t('nursery.discover.identityChanged');
      default:
        return device.assistants.length === 0 ? t('nursery.discover.nothingShared') : null;
    }
  };

  const hostProblem = problem ?? status?.problem ?? null;

  return (
    <Dialog open={open} size="md" onOpenChange={(nextOpen) => { if (!nextOpen) onClose(); }}>
      <DialogHeader>
        <DialogHeading>
          <DialogTitle>{t('nursery.discover.title')}</DialogTitle>
          <DialogDescription>{t('nursery.discover.description')}</DialogDescription>
        </DialogHeading>
        <DialogClose />
      </DialogHeader>
      <DialogBody className="discover-panel" data-testid="discover-panel">
        {hostProblem ? (
          <Alert
            tone="warning"
            message={t('nursery.discover.problem')}
            description={(
              <span className="discover-panel__problem" data-openbitfun-component="discover-panel" data-openbitfun-part="problem">
                <span>{hostProblem}</span>
                {settingsButton}
              </span>
            )}
          />
        ) : null}
        {requestProblem ? (
          <Alert tone="error" message={requestProblem} data-testid="discover-request-problem" />
        ) : null}

        {devices.length === 0 ? (
          searching && !hostProblem ? (
            <Empty
              className="discover-panel__empty"
              icon={<Spinner size="md" />}
              title={t('nursery.discover.searching')}
            />
          ) : (
            <Empty
              className="discover-panel__empty"
              title={t('nursery.discover.emptyTitle')}
              description={t('nursery.discover.emptyHint')}
              actions={hostProblem ? undefined : settingsButton}
              data-testid="discover-empty"
            />
          )
        ) : (
          <ul className="discover-panel__devices" data-openbitfun-component="discover-panel" data-openbitfun-part="devices">
            {devices.map((device) => {
              const note = deviceNote(device);
              return (
                <li key={device.publicKey} className="discover-panel__device" data-openbitfun-component="discover-panel" data-openbitfun-part="device" data-testid="discover-device">
                  <div className="discover-panel__device-head" data-openbitfun-component="discover-panel" data-openbitfun-part="deviceHead">
                    <span className="discover-panel__device-name" data-openbitfun-component="discover-panel" data-openbitfun-part="deviceName">{device.displayName}</span>
                    <span className="discover-panel__meta" data-openbitfun-component="discover-panel" data-openbitfun-part="meta">
                      {t('nursery.discover.device', { device: device.deviceName, code: device.fingerprint })}
                    </span>
                    {device.addedAddress ? (
                      <span className="discover-panel__meta discover-panel__added" data-openbitfun-component="discover-panel" data-openbitfun-part="added" data-testid="discover-device-added">
                        {t('nursery.discover.addedBadge', { address: shownAddress(device.addedAddress) })}
                        <Button
                          size="xs"
                          variant="text"
                          onClick={() => {
                            setMoveProblem(null);
                            setMovingDevice({ deviceId: device.deviceId, address: shownAddress(device.addedAddress ?? '') });
                          }}
                          data-testid="discover-device-move"
                        >
                          {t('nursery.discover.changeAddress')}
                        </Button>
                        <Button size="xs" variant="text" onClick={() => { void removeDevice(device); }} data-testid="discover-device-remove">
                          {t('nursery.discover.removeDevice')}
                        </Button>
                      </span>
                    ) : null}
                  </div>
                  {movingDevice?.deviceId === device.deviceId ? (
                    <span className="discover-panel__name-edit" data-openbitfun-component="discover-panel" data-openbitfun-part="nameEdit">
                      <Input
                        size="sm"
                        value={movingDevice.address}
                        maxLength={64}
                        autoFocus
                        aria-label={t('nursery.discover.byAddress.label')}
                        onValueChange={(address) => {
                          setMovingDevice({ deviceId: device.deviceId, address });
                          setMoveProblem(null);
                        }}
                        onKeyDown={(event) => {
                          if (event.key === 'Enter') void saveAddress();
                          if (event.key === 'Escape') setMovingDevice(null);
                        }}
                        data-testid="discover-device-address-input"
                      />
                      <Button size="sm" variant="primary" loading={movingBusy} disabled={!movingDevice.address.trim()} onClick={() => { void saveAddress(); }}>
                        {t('nursery.discover.nameSave')}
                      </Button>
                      <Button size="sm" variant="text" onClick={() => setMovingDevice(null)}>
                        {t('nursery.discover.nameCancel')}
                      </Button>
                    </span>
                  ) : null}
                  {movingDevice?.deviceId === device.deviceId && moveProblem ? (
                    <Alert tone="error" message={moveProblem} data-testid="discover-device-address-problem" />
                  ) : null}
                  {note ? <p className="discover-panel__meta" data-openbitfun-component="discover-panel" data-openbitfun-part="meta" data-testid="discover-device-note">{note}</p> : null}
                  {device.assistants.length > 0 ? (
                    <div className="discover-panel__assistants" data-openbitfun-component="discover-panel" data-openbitfun-part="assistants" role="list">
                      {device.assistants.map((assistant) => {
                        const key = accessKey(device, assistant);
                        const access = currentAccess(assistant.access, now);
                        const askable = access.state === 'available'
                          && device.status.state === 'ready';
                        return (
                          <Tooltip
                            key={key}
                            placement="bottom"
                            content={(
                              <span className="discover-panel__detail" data-openbitfun-component="discover-panel" data-openbitfun-part="detail">
                                <strong>{assistant.name}</strong>
                                {assistant.description ? <span>{assistant.description}</span> : null}
                                <span>{t('nursery.discover.from', { name: device.displayName, device: device.deviceName })}</span>
                                <span>{t('nursery.discover.code', { code: device.fingerprint })}</span>
                                <span>{t('nursery.discover.privacy')}</span>
                              </span>
                            )}
                          >
                            <button
                              type="button"
                              role="listitem"
                              data-openbitfun-component="discover-panel"
                              data-openbitfun-part="assistant"
                              className={[
                                'discover-panel__assistant',
                                `discover-panel__assistant--${access.state}`,
                              ].join(' ')}
                              disabled={!askable || asking !== null}
                              aria-busy={asking === key}
                              aria-label={`${assistant.name}, ${accessText(access)}`}
                              onClick={() => { void ask(device, assistant); }}
                              data-testid="discover-assistant"
                            >
                              <span className="discover-panel__avatar" data-openbitfun-component="discover-panel" data-openbitfun-part="avatar" aria-hidden="true">
                                <AssistantAvatar stableKey={key} name={assistant.name} size={44} />
                              </span>
                              <span className="discover-panel__assistant-name" data-openbitfun-component="discover-panel" data-openbitfun-part="assistantName">{assistant.name}</span>
                              <span className="discover-panel__meta" data-openbitfun-component="discover-panel" data-openbitfun-part="meta">{accessText(access)}</span>
                            </button>
                          </Tooltip>
                        );
                      })}
                    </div>
                  ) : null}
                </li>
              );
            })}
          </ul>
        )}
        <section className="discover-panel__other-subnet" data-openbitfun-component="discover-panel" data-openbitfun-part="otherSubnet">
          <ThisDeviceAddress />
          <AddByAddress />
        </section>
      </DialogBody>
      <DialogFooter className="discover-panel__footer">
        {editingName !== null ? (
          <span className="discover-panel__name-edit" data-openbitfun-component="discover-panel" data-openbitfun-part="nameEdit">
            <Input
              size="sm"
              value={editingName}
              maxLength={64}
              aria-label={t('nursery.discover.nameLabel')}
              onValueChange={setEditingName}
              onKeyDown={(event) => {
                if (event.key === 'Enter') void saveName();
                if (event.key === 'Escape') setEditingName(null);
              }}
              data-testid="discover-name-input"
            />
            <Button size="sm" variant="primary" loading={savingName} disabled={!editingName.trim()} onClick={() => { void saveName(); }}>
              {t('nursery.discover.nameSave')}
            </Button>
            <Button size="sm" variant="text" onClick={() => setEditingName(null)}>
              {t('nursery.discover.nameCancel')}
            </Button>
          </span>
        ) : (
          <span className="discover-panel__meta" data-openbitfun-component="discover-panel" data-openbitfun-part="meta">
            {status ? t('nursery.discover.shownAs', { name: status.displayName }) : null}
            {status ? (
              <Button size="xs" variant="text" onClick={() => setEditingName(status.displayName)} data-testid="discover-name-edit">
                {t('nursery.discover.nameEdit')}
              </Button>
            ) : null}
          </span>
        )}
        <span className="discover-panel__meta" data-openbitfun-component="discover-panel" data-openbitfun-part="meta">{t('nursery.discover.privacy')}</span>
      </DialogFooter>
    </Dialog>
  );
};

export default DiscoverPanel;
