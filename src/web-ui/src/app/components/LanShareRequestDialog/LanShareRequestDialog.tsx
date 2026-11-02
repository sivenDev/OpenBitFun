import React, { useEffect, useRef, useState } from 'react';
import {
  Button,
  Dialog,
  DialogBody,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogHeading,
  DialogTitle,
} from '@openbitfun/ui';
import { useI18n } from '@/infrastructure/i18n/hooks/useI18n';
import { lanShareAPI, type ShareAnswer, type ShareRequest } from '@/infrastructure/api/service-api/LanShareAPI';
import { systemAPI } from '@/infrastructure/api/service-api/SystemAPI';
import { secondsLeft, subscribeLanShare, useLanShareStore } from '@/infrastructure/lan-share/lanShareStore';
import { useNotification } from '@/shared/notification-system';
import { createLogger } from '@/shared/utils/logger';
import './LanShareRequestDialog.scss';

const log = createLogger('LanShareRequestDialog');
const TICK_MS = 250;
/**
 * How long the answers stay disabled after a request appears, so a click
 * meant for the previous request, or a request swapped in by a flood, is
 * never taken as consent.
 */
const ARM_DELAY_MS = 1000;

/**
 * Asks the owner about each request from another device, one at a time,
 * with the time left to answer. A request that runs out is declined by the
 * host; the dialog then moves on.
 */
const LanShareRequestDialog: React.FC = () => {
  const { t } = useI18n('scenes/profile');
  const notification = useNotification();
  const requests = useLanShareStore((state) => state.requests);
  const [now, setNow] = useState(() => Date.now());
  const [answering, setAnswering] = useState<ShareAnswer | null>(null);
  const answered = useRef(new Set<string>());
  const seen = useRef(new Map<string, ShareRequest>());

  useEffect(() => subscribeLanShare(), []);

  // Tell the owner about new requests while the window is in the background,
  // and about requests that ran out without an answer.
  useEffect(() => {
    const current = new Map(requests.map((request) => [request.requestId, request]));
    for (const request of requests) {
      if (seen.current.has(request.requestId)) continue;
      if (typeof document !== 'undefined' && !document.hasFocus()) {
        void systemAPI.sendSystemNotification(
          t('nursery.share.request.title'),
          t('nursery.share.request.notificationBody', {
            name: request.peer.displayName,
            assistant: request.assistantName,
          }),
        ).catch((error: unknown) => log.debug('System notification failed', { error }));
      }
    }
    for (const [requestId, request] of seen.current) {
      if (current.has(requestId) || answered.current.has(requestId)) continue;
      if (request.deadlineMs <= Date.now() + TICK_MS) {
        notification.info(t('nursery.share.request.expired', { name: request.peer.displayName }));
      }
    }
    seen.current = current;
  }, [notification, requests, t]);

  const request = requests.find((candidate) => !answered.current.has(candidate.requestId));

  const shownId = request?.requestId;
  const [shown, setShown] = useState<{ requestId?: string; at: number }>({ at: 0 });

  useEffect(() => {
    if (!shownId) return undefined;
    const at = Date.now();
    setShown({ requestId: shownId, at });
    setNow(at);
    const timer = window.setInterval(() => setNow(Date.now()), TICK_MS);
    return () => window.clearInterval(timer);
  }, [shownId]);

  if (!request) return null;
  const seconds = secondsLeft(request.deadlineMs, now);
  const waiting = requests.filter((candidate) => !answered.current.has(candidate.requestId)).length - 1;
  const armed = shown.requestId === request.requestId && now - shown.at >= ARM_DELAY_MS;
  const locked = !armed || answering !== null || seconds === 0;

  const answer = async (requestId: string, choice: ShareAnswer) => {
    // A click rendered for another request, or before this one could be read.
    if (answering || requestId !== shownId || !armed || answered.current.has(requestId)) return;
    setAnswering(choice);
    try {
      await lanShareAPI.decide(requestId, choice);
      answered.current.add(requestId);
    } catch (error) {
      const reason = error instanceof Error ? error.message : String(error);
      log.warn('Share request not answered', { requestId, error: reason });
      notification.error(t('nursery.share.request.failed', { reason }));
      answered.current.add(requestId);
    } finally {
      setAnswering(null);
      void useLanShareStore.getState().refreshRequests();
    }
  };

  return (
    <Dialog
      key={request.requestId}
      open
      size="sm"
      role="alertdialog"
      closeOnPointerOutside={false}
      onOpenChange={() => { void answer(request.requestId, 'decline'); }}
    >
      <DialogHeader>
        <DialogHeading>
          <DialogTitle>{t('nursery.share.request.title')}</DialogTitle>
          {waiting > 0 ? (
            <DialogDescription>{t('nursery.share.request.queued', { count: waiting })}</DialogDescription>
          ) : null}
        </DialogHeading>
      </DialogHeader>
      <DialogBody className="lan-share-request" data-testid="lan-share-request">
        <p className="lan-share-request__message" data-openbitfun-component="lan-share-request" data-openbitfun-part="message">
          {t('nursery.share.request.message', {
            name: request.peer.displayName,
            device: request.peer.deviceName,
            assistant: request.assistantName,
          })}
        </p>
        <p className="lan-share-request__code" data-openbitfun-component="lan-share-request" data-openbitfun-part="code" data-testid="lan-share-request-code">
          {t('nursery.share.request.code', { code: request.peer.fingerprint })}
        </p>
        <p className="lan-share-request__hint" data-openbitfun-component="lan-share-request" data-openbitfun-part="hint">{t('nursery.share.request.hint')}</p>
      </DialogBody>
      <DialogFooter className="lan-share-request__footer">
        <Button
          size="sm"
          variant="text"
          tone="danger"
          disabled={locked}
          loading={answering === 'block'}
          onClick={() => { void answer(request.requestId, 'block'); }}
          data-testid="lan-share-request-block"
        >
          {t('nursery.share.request.block')}
        </Button>
        <span className="lan-share-request__actions" data-openbitfun-component="lan-share-request" data-openbitfun-part="actions">
          <Button
            size="sm"
            variant="fill"
            disabled={locked}
            loading={answering === 'decline'}
            onClick={() => { void answer(request.requestId, 'decline'); }}
            data-testid="lan-share-request-decline"
          >
            {t('nursery.share.request.decline')}
          </Button>
          <Button
            size="sm"
            variant="primary"
            disabled={locked}
            loading={answering === 'allow'}
            onClick={() => { void answer(request.requestId, 'allow'); }}
            data-testid="lan-share-request-allow"
          >
            {t('nursery.share.request.allow', { seconds })}
          </Button>
        </span>
      </DialogFooter>
    </Dialog>
  );
};

export default LanShareRequestDialog;
