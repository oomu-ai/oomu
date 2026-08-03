import { invoke } from "@/lib/invoke";
import type { ProjectId } from "@/lib/p0Contracts";

export type ProjectPolicy = "local_only" | "ask_before_cloud" | "allow_configured_cloud";

export type ProjectRecord = {
  projectId: ProjectId;
  name: string;
  description: string;
  dataPolicy: ProjectPolicy;
  instructions: string;
  archivedAtMs: number | null;
  createdAtMs: number;
  updatedAtMs: number;
  sourceCount: number;
  conversationCount: number;
  workflowCount: number;
  taskCount: number;
};

export type ProjectSource = {
  sourceId: string;
  projectId: ProjectId;
  sourceKind: "local_folder" | "knowledge_directory";
  canonicalPath: string;
  grantState: "active" | "revoked" | "unavailable";
  indexingState: "pending" | "indexing" | "ready" | "failed" | "revoked";
  fileCount: number;
  lastIndexedAtMs: number | null;
  failureCode: string | null;
  updatedAtMs: number;
};

type ProjectDeletionPreview = {
  projectId: ProjectId;
  conversationsToDetach: number;
  workflowsToDetach: number;
  schedulesToDetach: number;
  taskRunsToDetach: number;
  sourcesToRemove: number;
  userFilesToDelete: number;
  defaultAction: string;
};

export const projectApi = {
  list: () => invoke<ProjectRecord[]>("list_projects", { includeArchived: false }),
  create: (name: string, description: string, dataPolicy: ProjectPolicy) =>
    invoke<ProjectRecord>("create_project", { request: { name, description, dataPolicy } }),
  update: (projectId: string, name: string, description: string) =>
    invoke<ProjectRecord>("update_project", { request: { projectId, name, description } }),
  instructions: (projectId: string, instructions: string) =>
    invoke<ProjectRecord>("set_project_instructions", { request: { projectId, instructions } }),
  policy: (projectId: string, dataPolicy: ProjectPolicy) =>
    invoke<ProjectRecord>("set_project_policy", { request: { projectId, dataPolicy } }),
  archive: (projectId: string) =>
    invoke<ProjectRecord>("archive_project", { request: { projectId } }),
  previewDeletion: (projectId: string) =>
    invoke<ProjectDeletionPreview>("preview_project_deletion", { request: { projectId } }),
  delete: (projectId: string) =>
    invoke<ProjectDeletionPreview>("delete_project", {
      request: {
        projectId,
        permanentlyRemoveProjectRecord: true,
        detachDependents: true,
        deleteProjectFiles: true,
      },
    }),
  sources: (projectId: string) =>
    invoke<ProjectSource[]>("list_project_sources", { request: { projectId } }),
  chooseRoot: (projectId: string) =>
    invoke<ProjectSource | null>("choose_project_root", { request: { projectId } }),
  refreshSource: (projectId: string, sourceId: string) =>
    invoke<ProjectSource>("refresh_project_source", { request: { projectId, sourceId } }),
  revokeSource: (projectId: string, sourceId: string) =>
    invoke<ProjectSource>("revoke_project_source", { request: { projectId, sourceId } }),
};
