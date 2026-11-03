import React, { useState } from 'react';
import { Alert, Button, Icon, IconButton, Input, Tooltip } from '@openbitfun/ui';
import { useI18n } from '@/infrastructure/i18n/hooks/useI18n';
import {
  lanShareAPI,
  type AddressedDevice,
  type AddressOutcome,
} from '@/infrastructure/api/service-api/LanShareAPI';
import { useLanShareStore } from '@/infrastructure/lan-share/lanShareStore';
import { useNotification } from '@/shared/notification-system';
import { writeNativeClipboardText } from '@/shared/utils/nativeClipboard';
import { createLogger } from '@/shared/utils/logger';
import { useAddressProblemText } from './useAddressProblemText';
import './LanShareAddress.scss';

const log = createLogger('LanShareAddress');
const COPIED_MS = 2000;

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

async function copyText(text: string): Promise<boolean> {
  if (await writeNativeClipboardText(text)) return true;
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch (error) {
    log.warn('Address not copied', { error: message(error) });
    return false;
  }
}

/**
 * This device's addresses and device code, for people on another subnet to
 * add it by. Shown only while this device shares an assistant and listens.
 */
export const ThisDeviceAddress: React.FC = () => {
  const { t } = useI18n('scenes/profile');
  const status = useLanShareStore((state) => state.overview?.status);
  const [copied, setCopied] = useState<string | null>(null);
  const addresses = status?.addresses ?? [];
  if (!status || addresses.length === 0) return null;

  const copy = async (address: string) => {
    if (await copyText(address)) {
      setCopied(address);
      window.setTimeout(() => setCopied(null), COPIED_MS);
    }
  };

  return (
    <div className="lan-share-address__this-device" data-openbitfun-component="lan-share-address" data-openbitfun-part="thisDevice" data-testid="lan-share-this-device">
      <div className="lan-share-address__addresses" data-openbitfun-component="lan-share-address" data-openbitfun-part="addresses">
        <span className="lan-share-address__meta" data-openbitfun-component="lan-share-address" data-openbitfun-part="meta">
          {t('nursery.lanAddress.label')}
        </span>
        {addresses.map((address) => (
          <span key={address} className="lan-share-address__address" data-openbitfun-component="lan-share-address" data-openbitfun-part="address" data-testid="lan-share-this-address">
            {address}
            <Tooltip content={copied === address ? t('nursery.lanAddress.copied') : t('nursery.lanAddress.copy')}>
              <IconButton
                size="xs"
                variant="quiet"
                icon={<Icon name={copied === address ? 'check-line' : 'duplicate'} size="sm" />}
                onClick={() => { void copy(address); }}
                aria-label={t('nursery.lanAddress.copy')}
              />
            </Tooltip>
          </span>
        ))}
        <span className="lan-share-address__meta" data-openbitfun-component="lan-share-address" data-openbitfun-part="meta">
          {t('nursery.lanAddress.code', { code: status.fingerprint })}
        </span>
      </div>
      <p className="lan-share-address__meta" data-openbitfun-component="lan-share-address" data-openbitfun-part="meta">
        {t('nursery.lanAddress.hint')}
      </p>
    </div>
  );
};

/**
 * Adds a device Bonjour cannot see, on another subnet, by the address its
 * owner reads out. The device code is compared before anything is kept.
 */
export const AddByAddress: React.FC = () => {
  const { t } = useI18n('scenes/profile');
  const notification = useNotification();
  const problemText = useAddressProblemText();
  const [open, setOpen] = useState(false);
  const [input, setInput] = useState('');
  const [busy, setBusy] = useState(false);
  const [found, setFound] = useState<AddressedDevice | null>(null);
  const [problem, setProblem] = useState<string | null>(null);

  const reset = () => {
    setOpen(false);
    setInput('');
    setFound(null);
    setProblem(null);
  };

  const settle = (outcome: AddressOutcome): AddressedDevice | null => {
    if (outcome.state === 'problem') {
      setProblem(problemText(outcome.problem));
      return null;
    }
    setProblem(null);
    return outcome.device;
  };

  const look = async () => {
    if (!input.trim() || busy) return;
    setBusy(true);
    setFound(null);
    try {
      setFound(settle(await lanShareAPI.probeAddress(input)));
    } catch (error) {
      setProblem(t('nursery.discover.byAddress.failed', { reason: message(error) }));
    } finally {
      setBusy(false);
    }
  };

  const add = async (device: AddressedDevice) => {
    if (busy) return;
    setBusy(true);
    try {
      const added = settle(await lanShareAPI.addDevice(device.address, device.publicKey));
      if (added) {
        notification.success(t('nursery.discover.byAddress.added', { name: added.displayName }));
        reset();
        void useLanShareStore.getState().refreshDiscovered();
      }
    } catch (error) {
      setProblem(t('nursery.discover.byAddress.failed', { reason: message(error) }));
    } finally {
      setBusy(false);
    }
  };

  if (!open) {
    return (
      <Button size="sm" variant="text" onClick={() => setOpen(true)} data-testid="discover-by-address-open">
        {t('nursery.discover.byAddress.open')}
      </Button>
    );
  }

  return (
    <div className="lan-share-address__form" data-openbitfun-component="lan-share-address" data-openbitfun-part="form" data-testid="discover-by-address">
      <p className="lan-share-address__meta" data-openbitfun-component="lan-share-address" data-openbitfun-part="meta">
        {t('nursery.discover.byAddress.hint')}
      </p>
      <div className="lan-share-address__row" data-openbitfun-component="lan-share-address" data-openbitfun-part="row">
        <Input
          size="sm"
          value={input}
          maxLength={64}
          autoFocus
          placeholder={t('nursery.discover.byAddress.placeholder')}
          aria-label={t('nursery.discover.byAddress.label')}
          onValueChange={(value) => {
            setInput(value);
            setFound(null);
            setProblem(null);
          }}
          onKeyDown={(event) => {
            if (event.key === 'Enter') void look();
            if (event.key === 'Escape') reset();
          }}
          data-testid="discover-by-address-input"
        />
        <Button size="sm" variant="fill" loading={busy && !found} disabled={!input.trim() || busy} onClick={() => { void look(); }} data-testid="discover-by-address-look">
          {t('nursery.discover.byAddress.look')}
        </Button>
        <Button size="sm" variant="text" onClick={reset}>
          {t('nursery.discover.byAddress.cancel')}
        </Button>
      </div>
      {problem ? <Alert tone="error" message={problem} data-testid="discover-by-address-problem" /> : null}
      {found ? (
        <div className="lan-share-address__found" data-openbitfun-component="lan-share-address" data-openbitfun-part="found" data-testid="discover-by-address-found">
          <span>{t('nursery.discover.byAddress.found', { name: found.displayName, device: found.deviceName })}</span>
          <strong data-testid="discover-by-address-code">
            {t('nursery.discover.byAddress.compare', { code: found.fingerprint })}
          </strong>
          <span className="lan-share-address__meta" data-openbitfun-component="lan-share-address" data-openbitfun-part="meta">
            {found.added
              ? t('nursery.discover.byAddress.alreadyAdded')
              : found.assistants > 0
                ? t('nursery.discover.byAddress.sharing', { count: found.assistants })
                : t('nursery.discover.byAddress.nothingShared')}
          </span>
          {found.added ? null : (
            <span className="lan-share-address__actions" data-openbitfun-component="lan-share-address" data-openbitfun-part="actions">
              <Button size="sm" variant="primary" loading={busy} onClick={() => { void add(found); }} data-testid="discover-by-address-add">
                {t('nursery.discover.byAddress.add')}
              </Button>
              <Button size="sm" variant="text" disabled={busy} onClick={() => setFound(null)}>
                {t('nursery.discover.byAddress.mismatch')}
              </Button>
            </span>
          )}
        </div>
      ) : null}
    </div>
  );
};
