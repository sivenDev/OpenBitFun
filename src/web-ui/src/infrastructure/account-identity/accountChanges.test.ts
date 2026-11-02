import { describe, expect, it, vi } from 'vitest';
import type { AccountIdentitySnapshot } from './AccountIdentityService';
import { onSignedInAccountChanged } from './accountChanges';
import type { MarketMe } from '@/infrastructure/api/service-api/AccountIdentityAPI';

function me(login: string): MarketMe {
  return { user: { accountId: `id-${login}`, githubId: 1, login } } as unknown as MarketMe;
}

function source(initial: AccountIdentitySnapshot) {
  let snapshot = initial;
  const listeners = new Set<(next: AccountIdentitySnapshot) => void>();
  return {
    getSnapshot: () => snapshot,
    subscribe: (listener: (next: AccountIdentitySnapshot) => void) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    set(next: AccountIdentitySnapshot) {
      snapshot = next;
      listeners.forEach(listener => listener(next));
    },
  };
}

const signedOut: AccountIdentitySnapshot = { resolved: true, status: 'signed-out', me: null };
const signedIn = (login: string): AccountIdentitySnapshot => ({ resolved: true, status: 'signed-in', me: me(login) });

describe('onSignedInAccountChanged', () => {
  it('fires on sign-in, a switch of account and sign-out, but not on refreshes', () => {
    const identity = source(signedOut);
    const listener = vi.fn();
    onSignedInAccountChanged(listener, identity);

    identity.set({ resolved: true, status: 'authorizing', me: null });
    expect(listener).not.toHaveBeenCalled();
    identity.set(signedIn('ann'));
    identity.set(signedIn('ann'));
    expect(listener).toHaveBeenCalledTimes(1);
    identity.set(signedIn('bea'));
    identity.set(signedOut);
    expect(listener).toHaveBeenCalledTimes(3);
  });

  it('takes the first resolution as the account the page loaded with', () => {
    const identity = source({ resolved: false, status: 'loading', me: null });
    const listener = vi.fn();
    onSignedInAccountChanged(listener, identity);

    identity.set(signedIn('ann'));
    expect(listener).not.toHaveBeenCalled();
    identity.set(signedOut);
    expect(listener).toHaveBeenCalledTimes(1);
  });
});
