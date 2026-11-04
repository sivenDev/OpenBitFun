/**
 * What the person opened, closed, or asked to see in full in the
 * navigation's task lists, for as long as the app runs.
 */

/** Groups opened or closed by hand; a group nobody touched opens by its own rule. */
export const openedGroups = new Map<string, boolean>();

/** Lists whose rows past the limit were asked for. */
export const fullLists = new Set<string>();

/** Test hook: forget what was opened or shown in full. */
export function resetNavListMemory(): void {
  openedGroups.clear();
  fullLists.clear();
}
