import { invokeTauri } from "@/shared/api/tauri";

/** Backlog connection state. The token stays in the OS keyring (Rust side). */
export type BacklogConnectionStatus = {
  connected: boolean;
  baseUrl?: string;
  userName?: string;
};

/** Backlog project reference — guid addresses everything server-side. */
export type BacklogProjectRef = {
  guid: string;
  path: string;
};

export function getBacklogStatus(): Promise<BacklogConnectionStatus> {
  return invokeTauri<BacklogConnectionStatus>("backlog_status");
}

/** Connect with a pasted token, or mint one via email + password login. */
export function connectBacklog(input: {
  baseUrl: string;
  token?: string;
  email?: string;
  password?: string;
}): Promise<BacklogConnectionStatus> {
  return invokeTauri<BacklogConnectionStatus>("backlog_connect", { input });
}

export function disconnectBacklog(): Promise<void> {
  return invokeTauri<void>("backlog_disconnect");
}

export function listBacklogProjects(): Promise<BacklogProjectRef[]> {
  return invokeTauri<BacklogProjectRef[]>("backlog_list_projects");
}

/** Raw task rows for a project; the projects feature maps them to issues. */
export function listBacklogTasks(projectGuid: string): Promise<unknown> {
  return invokeTauri<unknown>("backlog_list_tasks", { projectGuid });
}

export function createBacklogTask(
  projectGuid: string,
  title: string,
  description?: string,
): Promise<{ id: string }> {
  return invokeTauri<{ id: string }>("backlog_create_task", {
    description,
    projectGuid,
    title,
  });
}

export function createBacklogTaskComment(
  projectGuid: string,
  taskId: string,
  body: string,
): Promise<void> {
  return invokeTauri<void>("backlog_create_task_comment", {
    body,
    projectGuid,
    taskId,
  });
}

/**
 * Create/reuse a Backlog agent, grant it the project, mint a project-pinned
 * token; returns BACKLOG_URL / BACKLOG_TOKEN / BACKLOG_PROJECT_ID for
 * persona env vars.
 */
export function provisionBacklogAgentEnv(
  agentName: string,
  projectGuid: string,
): Promise<Record<string, string>> {
  return invokeTauri<Record<string, string>>("backlog_provision_agent_env", {
    agentName,
    projectGuid,
  });
}
