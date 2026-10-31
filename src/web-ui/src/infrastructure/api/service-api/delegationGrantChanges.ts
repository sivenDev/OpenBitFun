/**
 * Which assistants an account may use is changed from two places on the AI
 * collaboration page: an assistant's card and an account's row. Each tells
 * the other here so both show the same grants without a reload.
 */

type GrantsListener = (source: unknown) => void;

const listeners = new Set<GrantsListener>();

export function onDelegationGrantsChanged(listener: GrantsListener): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

/** `source` identifies the caller, which already shows its own change. */
export function notifyDelegationGrantsChanged(source?: unknown): void {
  listeners.forEach(listener => listener(source));
}
