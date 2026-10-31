/**
 * Copy keys in the `flow-chat` namespace that name a permission action, shared
 * by every surface that asks the owner to approve one.
 */
export const PERMISSION_ACTION_LABEL_KEYS: Record<string, string> = {
  read: 'permission.actions.read',
  edit: 'permission.actions.edit',
  bash: 'permission.actions.bash',
  git: 'permission.actions.git',
  computer_use: 'permission.actions.computerUse',
  websearch: 'permission.actions.webSearch',
  webfetch: 'permission.actions.webFetch',
  mcp: 'permission.actions.mcp',
  task: 'permission.actions.task',
  skill: 'permission.actions.skill',
  page_publish: 'permission.actions.pagePublish',
  page_deploy: 'permission.actions.pageDeploy',
  custom_tool: 'permission.actions.customTool',
  external_directory: 'permission.actions.externalDirectory',
};

/** The label of a permission action; `t` must read the `flow-chat` namespace. */
export function permissionActionLabel(action: string, t: (key: string) => string): string {
  return t(PERMISSION_ACTION_LABEL_KEYS[action] ?? 'permission.actions.other');
}
