import { invoke } from "@/lib/invoke";
import { artifactDocumentSchema, type ArtifactDocument } from "@/lib/artifacts/schema";

type ArtifactVerification = { structurallyVerifiedDocx: boolean; structurallyVerifiedPdf: boolean; visuallyVerifiedPdf: boolean; pageCount: number; warnings: string[]; rendererProbe: string };
export type ArtifactVersion = { version: number; revisionInstruction: string | null; status: string; document: ArtifactDocument; previewPages: string[]; verification: ArtifactVerification; provenance: unknown; docxBytes: number | null; pdfBytes: number | null; docxSha256: string | null; pdfSha256: string | null; builderIdentity: string; rendererIdentity: string | null; createdAtMs: number; completedAtMs: number | null; lastError: string | null };
export type ArtifactRecord = { artifactId: string; projectId: string; taskRunId: string; title: string; currentVersion: number; createdAtMs: number; updatedAtMs: number; versions: ArtifactVersion[] };

export const artifactApi = {
  list: (projectId?: string, taskRunId?: string) => invoke<ArtifactRecord[]>("list_artifacts", { request: { projectId: projectId || null, taskRunId: taskRunId || null } }),
  create: (projectId: string, taskRunId: string, document: ArtifactDocument) => invoke<ArtifactRecord>("create_artifact", { request: { projectId, taskRunId, document: artifactDocumentSchema.parse(document) } }),
  revise: (artifactId: string, projectId: string, taskRunId: string, instruction: string, document: ArtifactDocument) => invoke<ArtifactRecord>("revise_artifact", { request: { artifactId, projectId, taskRunId, instruction, document: artifactDocumentSchema.parse(document) } }),
  preview: (artifactId: string, version: number, page: number) => invoke<string>("get_artifact_preview_page", { request: { artifactId, version, page } }),
  chooseExport: (artifactId: string, version: number) => invoke<{ exportGrantId: string; directoryName: string; expiresAtMs: number } | null>("choose_artifact_export_destination", { request: { artifactId, version } }),
  export: (artifactId: string, version: number, exportGrantId: string, format: "docx" | "pdf" | "both") => invoke<{ exportedFiles: string[]; hashes: Record<string, string> }>("export_artifact", { request: { artifactId, version, exportGrantId, format } }),
};
