import type { ReactNode } from 'react';
import { Icon, OverflowText, Tooltip } from '@openbitfun/ui';
import { useI18n } from '@/infrastructure/i18n';
import { NavTaskStatusIcon, type NavTaskStatusKind } from './NavTaskStatus';

interface NavTaskRowProps {
  title: string;
  status: NavTaskStatusKind;
  /** The status in words; the status icon's tooltip says it. */
  statusLabel: string | null;
  /** Everything the row says, for its tooltip and accessible name. */
  said: string;
  /** Muted text after the title, such as who sent the task. */
  meta?: string | null;
  /** The task's session is the one being viewed. */
  selected: boolean;
  /** The row's menu is open, so its "..." button stays shown. */
  menuOpen?: boolean;
  className?: string;
  testId: string;
  onOpen: () => void;
  /** The row's "..." button, if it has a menu. */
  actions?: ReactNode;
}

/**
 * A task in the navigation, laid out like a session row. Unlike a session's,
 * its status stays in view on hover so its tooltip can be read: a "..."
 * button takes the cell after the status rather than the status's own.
 */
export function NavTaskRow({
  title, status, statusLabel, said, meta, selected, menuOpen = false, className, testId, onOpen, actions,
}: NavTaskRowProps) {
  return (
    <Tooltip content={said} placement="right" followCursor disabled={menuOpen}>
      <div
        role="button"
        tabIndex={0}
        className={[
          'openbitfun-nav-panel__inline-item',
          'nav-task-row',
          selected && 'is-active',
          menuOpen && 'is-menu-open',
          className,
        ].filter(Boolean).join(' ')}
        data-openbitfun-component="sessions-section"
        data-openbitfun-part="row"
        data-overflow-trigger
        data-openbitfun-state={[selected && 'active', menuOpen && 'menuOpen'].filter(Boolean).join(' ') || undefined}
        aria-current={selected ? 'true' : undefined}
        aria-label={said}
        onClick={onOpen}
        onKeyDown={(event) => {
          if (event.target !== event.currentTarget || (event.key !== 'Enter' && event.key !== ' ')) return;
          event.preventDefault();
          onOpen();
        }}
        data-testid={testId}
      >
        <span className="openbitfun-nav-panel__inline-item-main" data-openbitfun-component="sessions-section" data-openbitfun-part="rowMain">
          <span className="openbitfun-nav-panel__inline-item-copy">
            <span className="openbitfun-nav-panel__inline-item-primary">
              <span className="openbitfun-nav-panel__inline-item-title">
                <OverflowText behavior="marquee" title="" className="openbitfun-nav-panel__inline-item-label">{title}</OverflowText>
              </span>
              {meta ? (
                <OverflowText title="" className="openbitfun-nav-panel__inline-item-workspace-name">{meta}</OverflowText>
              ) : null}
            </span>
          </span>
        </span>
        <div className="openbitfun-nav-panel__inline-item-trailing">
          <span className="openbitfun-nav-panel__inline-item-status">
            <NavTaskStatusIcon kind={status} label={statusLabel} />
          </span>
          {actions}
        </div>
      </div>
    </Tooltip>
  );
}

/** "Show more +N" under a task list, or "Show less" once it shows all, as a session list has. */
export function NavShowMoreToggle({ hidden, showAll, onToggle, testId }: {
  hidden: number;
  showAll: boolean;
  onToggle: () => void;
  testId: string;
}) {
  const { t } = useI18n('common');
  return (
    <button
      type="button"
      className="openbitfun-nav-panel__inline-toggle"
      data-openbitfun-component="sessions-section"
      data-openbitfun-part="toggle"
      aria-label={showAll ? t('nav.sessions.showLess') : t('nav.sessions.showMore', { count: hidden })}
      onClick={onToggle}
      data-testid={testId}
    >
      <OverflowText className="openbitfun-nav-panel__inline-toggle-label">
        {showAll ? t('nav.sessions.showLess') : t('nav.sessions.showMoreLabel')}
      </OverflowText>
      {showAll ? null : (
        <span className="openbitfun-nav-panel__inline-toggle-count" aria-hidden>+{hidden}</span>
      )}
      <span className="openbitfun-nav-panel__inline-toggle-trailing">
        <Icon
          name={showAll ? 'chevron-up' : 'chevron-down'}
          size="xs"
          className="openbitfun-nav-panel__inline-toggle-chevron"
          aria-hidden
        />
      </span>
    </button>
  );
}
