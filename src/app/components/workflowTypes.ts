import { z } from "zod";

export const workflowActionKindSchema = z.enum([
  "file_read",
  "file_write",
  "file_list",
  "system_metric",
  "system_audit",
  "local_inference",
]);
