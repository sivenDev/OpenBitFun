/**
 * Session row menu actions, run against the rendered device surface.
 *
 * The live session list and the read-only device catalog share these: the
 * catalog renders the owning device first and then runs the same handler, so
 * both menus report success and failure identically.
 */

import { clearAssistantDefaultSession } from '@/app/components/NavPanel/sections/workspaces/assistantSessionPreferences';
import { flowChatManager } from '@/flow_chat/services/FlowChatManager';
import { resolveSessionDriverId } from '@/flow_chat/session-drivers/resolve';
import type { Session } from '@/flow_chat/types/flow-chat';
import { isSessionEmptyForCopy } from '@/flow_chat/utils/sessionCopy';
import { isOutcomeUnknownError } from '@/infrastructure/api/errors/TauriCommandError';
import { isSurfaceChangedError } from '@/infrastructure/peer-device/deviceSurface';
import { workspaceManager } from '@/infrastructure/services/business/workspaceManager';
import { notificationService } from '@/shared/notification-system';
import { createLogger } from '@/shared/utils/logger';
import { isSamePath } from '@/shared/utils/pathUtils';

const log = createLogger('SessionRowActions');

export type SessionRowTranslate = (key: string, options?: Record<string, unknown>) => string;

/** Archive a session; the archive module reports its own failures. */
export async function archiveSessionRow(sessionId: string, t: SessionRowTranslate): Promise<void> {
  try {
    await flowChatManager.archiveChatSession(sessionId);
    window.dispatchEvent(new CustomEvent('openbitfun:session-archived'));
    notificationService.success(t('nav.sessions.archivedAll', { count: 1 }), { duration: 3000 });
  } catch (err) {
    log.error('Failed to archive session', err);
  }
}

/**
 * Whether the menu offers deleting `session` outright: an assistant's own
 * top-level session that has sent nothing yet, so nothing is lost. Sessions
 * with content are archived first and deleted from the archive.
 */
export function canDeleteEmptySession(
  session: Session,
  { isAssistant, isChild, isRunning }: { isAssistant: boolean; isChild: boolean; isRunning: boolean },
): boolean {
  return isAssistant
    && !isChild
    && !isRunning
    && !session.isTransient
    && !session.importedSource
    && session.persistedStatus !== 'archived'
    && !session.parentSessionId
    && session.sessionKind !== 'subagent'
    && resolveSessionDriverId(session.sessionId, session) === 'local'
    && isSessionEmptyForCopy(session);
}

/**
 * Delete an empty assistant session, then stop naming it the assistant's
 * default session. The deletion stands when only the default fails to clear.
 */
export async function deleteEmptySessionRow(
  sessionId: string,
  assistantWorkspaceId: string,
  t: SessionRowTranslate,
): Promise<void> {
  try {
    await flowChatManager.deleteChatSession(sessionId);
  } catch (error) {
    if (isSurfaceChangedError(error)) return;
    log.error('Failed to delete empty session', { sessionId, error });
    notificationService.error(t('nav.sessions.deleteFailed'), { duration: 4000 });
    return;
  }
  try {
    await clearAssistantDefaultSession(assistantWorkspaceId, sessionId);
  } catch (error) {
    if (isSurfaceChangedError(error)) return;
    log.warn('Deleted session but failed to clear it as the assistant default', { sessionId, error });
    notificationService.warning(t('nav.sessions.defaultSaveFailed'), { duration: 4000 });
  }
}

/**
 * Copy the session into a new independent session. The copy is created by the
 * host and appears in the list; the current row and the active session stay
 * untouched.
 */
export async function duplicateSessionRow(sessionId: string, t: SessionRowTranslate): Promise<void> {
  try {
    const copied = await flowChatManager.duplicateChatSession(sessionId);
    if (copied.metadataProjected) {
      notificationService.success(
        t('nav.sessions.duplicateSuccess', { name: copied.sessionName }),
        { duration: 3000 },
      );
    } else {
      notificationService.warning(
        t('nav.sessions.duplicateCreatedRefreshFailed', { name: copied.sessionName }),
        { duration: 5000 },
      );
    }
  } catch (error) {
    if (isSurfaceChangedError(error)) return;
    log.error('Failed to copy session', { sessionId, error });
    notificationService.error(
      error instanceof Error && error.message
        ? error.message
        : t('nav.sessions.duplicateFailed'),
      { duration: 4000 },
    );
  }
}

/** Rename a session; an unknown outcome reloads the host's title. */
export async function renameSessionRow(sessionId: string, title: string, t: SessionRowTranslate): Promise<void> {
  try {
    await flowChatManager.renameChatSessionTitle(sessionId, title);
  } catch (err) {
    log.error('Failed to update session title', { sessionId, error: err });
    if (isOutcomeUnknownError(err)) {
      notificationService.warning(t('nav.sessions.renameOutcomeUnknown'), { duration: 6000 });
      try {
        await flowChatManager.reloadSessionTitle(sessionId);
      } catch (refreshError) {
        log.error('Failed to reload the session title after an unknown rename outcome', {
          sessionId,
          error: refreshError,
        });
      }
    }
  }
}

/**
 * A worktree directory is registered for execution but not opened, so the
 * session row is the only place it can be reached from. Opening it as a
 * workspace is an explicit user action; if the worktree happens to be open
 * already, activating it is the whole effect.
 */
export async function openWorktreeWorkspaceRow(worktreePath: string, t: SessionRowTranslate): Promise<void> {
  const opened = [...workspaceManager.getState().openedWorkspaces.values()].find(workspace =>
    isSamePath(workspace.rootPath ?? '', worktreePath),
  );
  try {
    if (opened) {
      await workspaceManager.setActiveWorkspace(opened.id);
      return;
    }
    await workspaceManager.openWorkspace(worktreePath);
  } catch (err) {
    log.error('Failed to open the worktree directory as a workspace', { worktreePath, error: err });
    notificationService.error(t('nav.sessions.openWorktreeWorkspaceFailed'), { duration: 3000 });
  }
}
