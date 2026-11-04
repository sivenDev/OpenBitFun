/** Where a task in the navigation stands, as the dot before its title shows it. */
export type NavTaskStatusKind =
  | 'approval'
  | 'queued'
  | 'running'
  | 'failed'
  | 'expired'
  | 'stopped'
  | 'done'
  | 'idle';

/** The dot before a task's title. Its words go in the row's trailing label. */
export function NavTaskStatusDot({ kind }: { kind: NavTaskStatusKind }) {
  return (
    <span
      className={`nav-task-status__dot is-${kind}`}
      data-openbitfun-component="sessions-section"
      data-openbitfun-part="statusMark"
      aria-hidden="true"
    />
  );
}

/** A short trailing label; waiting reads as a warning chip. */
export function NavTaskStatusLabel({ kind, text }: { kind: NavTaskStatusKind; text: string | null }) {
  if (!text) return null;
  return (
    <span className={['nav-task-status__label', kind === 'approval' && 'is-chip'].filter(Boolean).join(' ')}>
      {text}
    </span>
  );
}
