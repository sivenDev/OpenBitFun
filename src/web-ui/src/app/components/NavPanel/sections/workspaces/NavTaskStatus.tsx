import { Icon, Tooltip } from '@openbitfun/ui';
import { Ban, CircleAlert, Hand, Loader2, TimerOff } from 'lucide-react';
import '../sessions/SessionStatusIndicator.scss';

/** Where a task in the navigation stands, as the icon after its title shows it. */
export type NavTaskStatusKind =
  | 'approval'
  | 'queued'
  | 'running'
  | 'failed'
  | 'expired'
  | 'stopped'
  | 'done'
  | 'idle';

// The same marks session rows use, so a task reads like a session.
const presentation = {
  approval: { glyph: Hand, tone: 'warning' },
  queued: { name: 'clock', tone: 'secondary' },
  running: { glyph: Loader2, tone: 'info' },
  failed: { glyph: CircleAlert, tone: 'danger' },
  expired: { glyph: TimerOff, tone: 'secondary' },
  stopped: { glyph: Ban, tone: 'secondary' },
} as const;

/**
 * The icon after a task's title; settled tasks have none. `label` names it,
 * in its own tooltip and for assistive technology.
 */
export function NavTaskStatusIcon({ kind, label }: { kind: NavTaskStatusKind; label: string | null }) {
  if (kind === 'done' || kind === 'idle') return null;
  const appearance = presentation[kind];
  const icon = (
    <span
      className="session-status-indicator nav-task-status"
      data-openbitfun-component="sessions-section"
      data-openbitfun-part="statusMark"
      data-status={kind}
      role="img"
      aria-label={label ?? undefined}
    >
      <Icon
        {...('glyph' in appearance ? { glyph: appearance.glyph } : { name: appearance.name })}
        size="xs"
        tone={appearance.tone}
        aria-hidden="true"
      />
    </span>
  );
  return label ? <Tooltip content={label} placement="top">{icon}</Tooltip> : icon;
}
