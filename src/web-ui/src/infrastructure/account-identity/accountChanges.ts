import {
  accountIdentityKey,
  accountIdentityService,
  type AccountIdentitySnapshot,
} from './AccountIdentityService';

interface AccountChangeSource {
  getSnapshot(): AccountIdentitySnapshot;
  subscribe(listener: (snapshot: AccountIdentitySnapshot) => void): () => void;
}

/**
 * Calls `listener` whenever the signed-in account changes: on sign-in,
 * sign-out, or a switch to another account. Pages whose content belongs to
 * the account reload from it instead of waiting for a restart.
 */
export function onSignedInAccountChanged(
  listener: () => void,
  source: AccountChangeSource = accountIdentityService,
): () => void {
  const initial = source.getSnapshot();
  let key = initial.resolved ? accountIdentityKey(initial.me) : null;
  return source.subscribe(snapshot => {
    if (!snapshot.resolved) return;
    const next = accountIdentityKey(snapshot.me);
    if (key === null) {
      // The first resolution is what the page loaded with.
      key = next;
      return;
    }
    if (next === key) return;
    key = next;
    listener();
  });
}
