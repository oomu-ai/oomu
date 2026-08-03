import { z } from "zod";
import { invoke } from "@/lib/invoke";
import { presentationIrSchema, type PresentationIr } from "@/lib/artifacts/presentations/schema";

const presentationStatusSchema = z.enum(["building", "check_required", "ready", "failed"]);
const revisionScopeSchema = z.enum(["slide", "element", "narrative_section", "notes", "citations", "theme", "whole_presentation"]);
const issueSchema = z.object({
  issueId: z.string(), revision: z.number().int().positive(), slideId: z.string().nullable().optional(),
  code: z.string(), severity: z.enum(["info", "warning", "blocker"]), message: z.string(),
  objectId: z.string().nullable().optional(), evidenceRef: z.string().nullable().optional(),
}).strict();
const thumbnailSchema = z.object({ mediaType: z.literal("image/png"), bytesBase64: z.string().min(4), width: z.number().int().positive(), height: z.number().int().positive() }).strict();
const summarySchema = z.object({
  presentationId: z.string(), projectId: z.string(), taskId: z.string(), taskRunId: z.string(), artifactId: z.string(), title: z.string(),
  currentRevision: z.number().int().positive(), status: presentationStatusSchema, slideCount: z.number().int().nonnegative(), issueCount: z.number().int().nonnegative(), blockerCount: z.number().int().nonnegative(),
  structurallyVerified: z.boolean(), visuallyVerified: z.boolean(), exportable: z.boolean(), updatedAtMs: z.number().int().nonnegative(),
}).strict();
const filmstripSchema = z.object({ slideId: z.string(), position: z.number().int().nonnegative(), title: z.string(), layoutId: z.string(), thumbnail: thumbnailSchema.nullable().optional(), issueCount: z.number().int().nonnegative(), blockerCount: z.number().int().nonnegative() }).strict();
const verificationSchema = z.object({
  packageSha256: z.string().regex(/^[0-9a-f]{64}$/), structurallyVerified: z.boolean(), visuallyVerified: z.boolean(), exportable: z.boolean(), checkedAtMs: z.number().int().nonnegative(), renderer: z.string().nullable().optional(),
  checks: z.array(z.object({ code: z.string(), passed: z.boolean(), detail: z.string(), slideId: z.string().nullable().optional(), objectId: z.string().nullable().optional() }).strict()),
  issues: z.array(issueSchema),
}).strict();
const detailSchema = z.object({
  summary: summarySchema,
  selectedRevision: z.number().int().positive(),
  presentation: presentationIrSchema,
  revisionHistory: z.array(z.object({ revision: z.number().int().positive(), createdAtMs: z.number().int().nonnegative(), scope: revisionScopeSchema, changeSummary: z.string(), structurallyVerified: z.boolean(), visuallyVerified: z.boolean(), exportable: z.boolean() }).strict()),
  filmstrip: z.array(filmstripSchema),
  issues: z.array(issueSchema),
  notes: z.array(z.object({ slideId: z.string(), speakerNotes: z.string(), sourceRefs: z.array(z.string()) }).strict()),
  citations: z.array(z.object({ citationId: z.string(), slideId: z.string(), objectId: z.string().nullable().optional(), sourceRef: z.string(), evidenceRef: z.string(), label: z.string(), locator: z.string().nullable().optional() }).strict()),
  provenance: z.array(z.object({ slideId: z.string(), objectId: z.string(), sourceRef: z.string(), evidenceRef: z.string(), evidenceClass: z.enum(["model_assertion", "observed_result", "executed_mutation", "verified_postcondition", "signed_artifact"]) }).strict()),
  templateIdentity: z.object({ templateId: z.string().nullable().optional(), name: z.string(), imported: z.boolean(), fingerprintSha256: z.string(), masterIds: z.array(z.string()), layoutIds: z.array(z.string()) }).strict(),
  verification: verificationSchema,
}).strict();

export type PresentationReviewSummary = z.infer<typeof summarySchema>;
export type PresentationReviewDetail = z.infer<typeof detailSchema>;
type PresentationRevisionScope = z.infer<typeof revisionScopeSchema>;
export type PresentationReviewIssue = z.infer<typeof issueSchema>;
const registeredTemplateSchema = z.object({
  templateId: z.string().min(1), name: z.string().min(1), fingerprintSha256: z.string().regex(/^[0-9a-f]{64}$/),
  masterParts: z.array(z.string()), layoutParts: z.array(z.string()), slideCount: z.number().int().positive(),
  exactPartPreservationSupported: z.boolean(), taskSummaryCompatible: z.boolean(),
}).strict();
export type RegisteredPresentationTemplate = z.infer<typeof registeredTemplateSchema>;
const checkerReadinessSchema = z.object({
  status: z.enum(["ready", "not_installed", "not_qualified", "app_component_unavailable", "unsupported_platform"]),
  requiredVersion: z.string().min(1),
}).strict();
export type PresentationCheckerReadiness = z.infer<typeof checkerReadinessSchema>;

export const presentationApi = {
  create: async (projectId: string, taskId: string, taskRunId: string, title: string, presentation: PresentationIr) => detailSchema.parse(await invoke("create_presentation", { request: { projectId, taskId, taskRunId, title, presentation: presentationIrSchema.parse(presentation) } })),
  list: async (projectId?: string) => z.array(summarySchema).parse(await invoke("list_presentation_reviews", { request: { projectId: projectId || null } })),
  get: async (presentationId: string, revision?: number) => detailSchema.parse(await invoke("get_presentation_review", { request: { presentationId, revision: revision ?? null } })),
  checkerReadiness: async () => checkerReadinessSchema.parse(await invoke("get_presentation_checker_readiness")),
  openCheckerDownload: async () => invoke("open_presentation_checker_download"),
  preview: async (presentationId: string, revision: number) => z.object({ presentationId: z.string(), revision: z.number().int().positive(), filmstrip: z.array(filmstripSchema), issues: z.array(issueSchema), rendererUnavailable: z.boolean() }).strict().parse(await invoke("get_presentation_preview", { request: { presentationId, revision } })),
  inspectTemplate: async (projectId: string, taskId: string, taskRunId: string) => registeredTemplateSchema.nullable().parse(await invoke("inspect_presentation_template", { request: { projectId, taskId, taskRunId } })),
  revise: async (presentationId: string, expectedRevision: number, scope: PresentationRevisionScope, targetSlideIds: string[], changeSummary: string, presentation: PresentationIr, targetObjectIds: string[] = []) => detailSchema.parse(await invoke("revise_presentation_scope", { request: { presentationId, expectedRevision, scope, targetSlideIds, targetObjectIds, changeSummary, presentation: presentationIrSchema.parse(presentation) } })),
  recheck: async (presentationId: string, expectedRevision: number) => detailSchema.parse(await invoke("recheck_presentation_revision", { request: { presentationId, expectedRevision } })),
  chooseExport: async (presentationId: string, revision: number, suggestedName: string) => z.object({ grantToken: z.string().min(16), displayName: z.string(), expiresAtMs: z.number().int().positive() }).strict().nullable().parse(await invoke("choose_presentation_export_destination", { request: { presentationId, revision, suggestedName } })),
  export: async (presentationId: string, revision: number, grantToken: string) => z.object({ presentationId: z.string(), revision: z.number().int().positive(), displayName: z.string(), sha256: z.string().regex(/^[0-9a-f]{64}$/), receiptId: z.string() }).strict().parse(await invoke("export_presentation_revision", { request: { presentationId, revision, grantToken } })),
};
