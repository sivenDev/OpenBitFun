/**
 * Sessions of tasks other devices and accounts delegated to an assistant
 * here, as this window follows them. The task's runner owns the session and
 * its event log; this store only holds what the owner's read-only view has
 * replayed so far. Nothing here is persisted: reopening a task replays it.
 */

import { create } from 'zustand';

export interface DelegatedSessionEntry {
  jobId: string;
  sessionId: string;
  /** Byte cursor of the task's event log replayed so far. */
  cursor: number;
  state: string;
  /** Actions the task waits on, in the runner's permission request shape. */
  pendingPermissions: Array<Record<string, unknown>>;
}

interface DelegatedSessionStoreState {
  bySession: Record<string, DelegatedSessionEntry>;
  register: (entry: DelegatedSessionEntry) => void;
  update: (sessionId: string, update: Partial<Omit<DelegatedSessionEntry, 'jobId' | 'sessionId'>>) => void;
  remove: (sessionId: string) => void;
}

export const delegatedSessionStore = create<DelegatedSessionStoreState>()(set => ({
  bySession: {},
  register: entry => set(state => (
    state.bySession[entry.sessionId] ? state : { bySession: { ...state.bySession, [entry.sessionId]: entry } }
  )),
  update: (sessionId, update) => set(state => {
    const current = state.bySession[sessionId];
    return current ? { bySession: { ...state.bySession, [sessionId]: { ...current, ...update } } } : state;
  }),
  remove: sessionId => set(state => {
    if (!state.bySession[sessionId]) return state;
    const { [sessionId]: _removed, ...rest } = state.bySession;
    return { bySession: rest };
  }),
}));

export function delegatedSessionEntry(sessionId: string): DelegatedSessionEntry | undefined {
  return delegatedSessionStore.getState().bySession[sessionId];
}
