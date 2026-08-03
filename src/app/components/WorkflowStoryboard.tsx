"use client";

import { useEffect, useMemo, useRef, useState, type RefObject } from "react";
import { useI18n } from "@/context/I18nContext";
import { useHumanTrust } from "@/lib/utils/trustUtils";
import {
  McpSchemaFields,
} from "./McpSchemaFields";
import {
  addConditionalBranch,
  asRecord,
  capabilityActionsForAddStep,
  insertCapabilityStep,
  removeWorkflowIrNode,
  updateWorkflowIrEdgeTarget,
  updateWorkflowIrNode,
} from "./workflowStoryboardModel";
export { addConditionalBranch } from "./workflowStoryboardModel";
import type {
  CapabilityCatalog,
  WorkflowCapabilityAction,
} from "./workflowCapabilityCatalog";
import {
  type WorkflowIr,
  type WorkflowIrEdge,
  type WorkflowIrNode,
} from "./workflowIr";
import { firstSentenceForWorkflowPreview } from "./workflowPreviewText";

type TranslateFn = (key: string, variables?: Record<string, string | number>) => string;
type SelectFieldOption = string | { label: string; value: string };

export type StoryboardNature = "read" | "think" | "act" | "approve";

type StoryboardBranch = {
  id: string;
  label: string;
  port: string;
  targetId: string;
  targetLabel: string;
};

type StoryboardItem = {
  branches: StoryboardBranch[];
  detail: string;
  id: string;
  index: number;
  nature: StoryboardNature;
  natureLabel: string;
  node: WorkflowIrNode;
  title: string;
};

type WorkflowStoryboardProps = {
  catalog?: CapabilityCatalog | null;
  editable?: boolean;
  onWorkflowIrChange?: (workflowIr: WorkflowIr) => void;
  workflowIr: WorkflowIr;
};

export function WorkflowStoryboard({
  catalog,
  editable = false,
  onWorkflowIrChange,
  workflowIr,
}: WorkflowStoryboardProps) {
  const { t } = useI18n();
  const [expandedNodeId, setExpandedNodeId] = useState<string | null>(null);
  const [addAfterNodeId, setAddAfterNodeId] = useState<string | null>(null);
  const [branchAfterNodeId, setBranchAfterNodeId] = useState<string | null>(null);
  const [branchCondition, setBranchCondition] = useState(
    t("workflows.storyboard.default_branch_condition"),
  );
  const [editorError, setEditorError] = useState<string | null>(null);
  const expandedPanelRef = useRef<HTMLDivElement>(null);
  const items = useMemo(
    () => buildWorkflowStoryboardModel(workflowIr, t),
    [workflowIr, t],
  );
  const addableActions = useMemo(
    () => capabilityActionsForAddStep(catalog).slice(0, 10),
    [catalog],
  );
  const targetOptions = useMemo(
    () => workflowIr.nodes.map((node) => ({ id: node.id, label: node.label })),
    [workflowIr.nodes],
  );
  const outgoingCounts = useMemo(() => {
    const counts = new Map<string, number>();
    workflowIr.edges.forEach((edge) => {
      counts.set(edge.sourceNodeId, (counts.get(edge.sourceNodeId) ?? 0) + 1);
    });
    return counts;
  }, [workflowIr.edges]);

  useEffect(() => {
    if (expandedNodeId) {
      expandedPanelRef.current?.focus();
    }
  }, [expandedNodeId]);

  function commit(next: WorkflowIr, status: string) {
    setEditorError(null);
    onWorkflowIrChange?.(next);
    setExpandedNodeId((current) => current ?? next.nodes[0]?.id ?? null);
    return status;
  }

  function tryMutation(factory: () => WorkflowIr, status: string) {
    try {
      commit(factory(), status);
    } catch (error) {
      setEditorError(friendlyStoryboardError(error, t));
    }
  }

  function updateNode(nodeId: string, patch: Partial<WorkflowIrNode>) {
    tryMutation(
      () => updateWorkflowIrNode(workflowIr, nodeId, patch),
      t("workflows.storyboard.updated_status"),
    );
  }

  function updateEdgeTarget(edgeId: string, targetNodeId: string) {
    tryMutation(
      () => updateWorkflowIrEdgeTarget(workflowIr, edgeId, targetNodeId),
      t("workflows.storyboard.branch_updated_status"),
    );
  }

  function insertAction(afterNodeId: string, action: WorkflowCapabilityAction) {
    tryMutation(
      () => insertCapabilityStep(workflowIr, afterNodeId, action),
      t("workflows.storyboard.added_step_status", { name: action.title }),
    );
    setAddAfterNodeId(null);
    setExpandedNodeId(null);
  }

  function addBranch(afterNodeId: string) {
    tryMutation(
      () =>
        addConditionalBranch(workflowIr, afterNodeId, branchCondition, {
          branchLabel: t("workflows.storyboard.generated_branch_label"),
          defaultCondition: t("workflows.storyboard.generated_branch_condition"),
          fallbackLabel: t("workflows.storyboard.generated_otherwise_label"),
          fallbackObjective: t("workflows.storyboard.generated_otherwise_objective"),
        }),
      t("workflows.storyboard.branch_added_status"),
    );
    setBranchAfterNodeId(null);
    setExpandedNodeId(null);
  }

  function removeNode(nodeId: string) {
    tryMutation(
      () => removeWorkflowIrNode(workflowIr, nodeId),
      t("workflows.storyboard.removed_step_status"),
    );
    setExpandedNodeId(null);
  }

  return (
    <section className="flex min-h-0 flex-col">
      <div className="mb-4 flex flex-col gap-1">
        <h2 className="text-sm font-semibold text-[var(--foreground)]">
          {t("workflows.storyboard.title")}
        </h2>
        <p className="text-xs leading-5 text-[var(--foreground-muted)]">
          {editable
            ? t("workflows.storyboard.editable_description")
            : t("workflows.storyboard.description")}
        </p>
      </div>

      {editorError && (
        <div
          className="mb-3 rounded-[var(--radius-sm)] border border-[var(--destructive)]/30 bg-[var(--destructive-background)] px-3 py-2 text-xs leading-5 text-[var(--destructive)]"
          role="alert"
        >
          {editorError}
        </div>
      )}

      <ol
        aria-label={t("workflows.storyboard.steps_aria")}
        className="flex flex-col gap-3"
      >
        {items.map((item) => {
          const isExpanded = expandedNodeId === item.id;
          const canInsertAfter =
            editable &&
            item.node.kind !== "output" &&
            (outgoingCounts.get(item.id) ?? 0) === 1;
          const canRemove =
            editable &&
            item.node.kind !== "input" &&
            item.node.kind !== "output" &&
            (workflowIr.edges.filter((edge) => edge.sourceNodeId === item.id).length === 1);
          return (
            <li
              className={`rounded-[var(--radius-md)] border p-4 ${
                item.nature === "approve"
                  ? "border-[var(--border-strong)] bg-[var(--accent-background)]"
                  : "border-[var(--border-soft)] bg-[var(--background)]"
              }`}
              key={item.id}
            >
              <div className="flex items-start gap-3">
                <span className="grid h-8 w-8 shrink-0 place-items-center rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--background)] text-xs font-semibold text-[var(--foreground-muted)]">
                  {item.index}
                </span>
                <div className="min-w-0 flex-1">
                  <button
                    aria-controls={`workflow-step-editor-${item.id}`}
                    aria-expanded={editable ? isExpanded : undefined}
                    className="block w-full text-left"
                    onClick={() =>
                      editable
                        ? setExpandedNodeId((current) =>
                            current === item.id ? null : item.id,
                          )
                        : undefined
                    }
                    type="button"
                  >
                    <div className="flex flex-wrap items-center gap-2">
                      <h3 className="min-w-0 text-sm font-semibold text-[var(--foreground)]">
                        {item.title}
                      </h3>
                      <span className="inline-flex items-center gap-1 rounded-full border border-[var(--border-soft)] bg-[var(--background)] px-2 py-0.5 text-[11px] font-medium text-[var(--foreground-muted)]">
                        <WorkflowNatureIcon nature={item.nature} />
                        {item.natureLabel}
                      </span>
                    </div>
                    <p className="mt-2 text-xs leading-5 text-[var(--foreground-muted)]">
                      {item.detail}
                    </p>
                  </button>

                  {item.branches.length > 0 && (
                    <BranchList
                      branches={item.branches}
                      editable={editable}
                      onTargetChange={updateEdgeTarget}
                      targetOptions={targetOptions}
                    />
                  )}

                  {editable && isExpanded && (
                    <StepEditor
                      canInsertAfter={canInsertAfter}
                      canRemove={canRemove}
                      isAddingStep={addAfterNodeId === item.id}
                      isBranching={branchAfterNodeId === item.id}
                      node={item.node}
                      onAddBranch={() => setBranchAfterNodeId(item.id)}
                      onAddStep={() => setAddAfterNodeId(item.id)}
                      onBranchCancel={() => setBranchAfterNodeId(null)}
                      onBranchCommit={() => addBranch(item.id)}
                      onBranchConditionChange={setBranchCondition}
                      onInsertAction={(action) => insertAction(item.id, action)}
                      onRemove={() => removeNode(item.id)}
                      onUpdate={(patch) => updateNode(item.id, patch)}
                      panelId={`workflow-step-editor-${item.id}`}
                      panelRef={expandedPanelRef}
                      stepActions={addableActions}
                      branchCondition={branchCondition}
                    />
                  )}
                </div>
              </div>
            </li>
          );
        })}
      </ol>
    </section>
  );
}

function BranchList({
  branches,
  editable,
  onTargetChange,
  targetOptions,
}: {
  branches: StoryboardBranch[];
  editable: boolean;
  onTargetChange: (edgeId: string, targetNodeId: string) => void;
  targetOptions: { id: string; label: string }[];
}) {
  const { t } = useI18n();
  return (
    <div className="mt-3 rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--background)] p-3">
      <p className="text-[11px] font-semibold uppercase text-[var(--foreground-subtle)]">
        {t("workflows.storyboard.branches")}
      </p>
      <div className="mt-2 grid gap-2">
        {branches.map((branch) => (
          <div
            className="grid gap-2 rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--accent-background)] px-3 py-2 text-xs sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)] sm:items-center"
            key={branch.id}
          >
            <span className="min-w-0 truncate font-medium text-[var(--foreground)]">
              {branch.label}
            </span>
            {editable ? (
              <select
                aria-label={t("workflows.storyboard.branch_target_select", {
                  label: branch.label,
                })}
                className="min-w-0 rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--background)] px-2 py-1.5 text-xs text-[var(--foreground)]"
                onChange={(event) => onTargetChange(branch.id, event.target.value)}
                value={branch.targetId}
              >
                {targetOptions.map((target) => (
                  <option key={target.id} value={target.id}>
                    {target.label}
                  </option>
                ))}
              </select>
            ) : (
              <span className="min-w-0 truncate text-[var(--foreground-muted)]">
                {t("workflows.storyboard.branch_target", {
                  target: branch.targetLabel,
                })}
              </span>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}

function StepEditor({
  branchCondition,
  canInsertAfter,
  canRemove,
  isAddingStep,
  isBranching,
  node,
  onAddBranch,
  onAddStep,
  onBranchCancel,
  onBranchCommit,
  onBranchConditionChange,
  onInsertAction,
  onRemove,
  onUpdate,
  panelId,
  panelRef,
  stepActions,
}: {
  branchCondition: string;
  canInsertAfter: boolean;
  canRemove: boolean;
  isAddingStep: boolean;
  isBranching: boolean;
  node: WorkflowIrNode;
  onAddBranch: () => void;
  onAddStep: () => void;
  onBranchCancel: () => void;
  onBranchCommit: () => void;
  onBranchConditionChange: (value: string) => void;
  onInsertAction: (action: WorkflowCapabilityAction) => void;
  onRemove: () => void;
  onUpdate: (patch: Partial<WorkflowIrNode>) => void;
  panelId: string;
  panelRef: RefObject<HTMLDivElement | null>;
  stepActions: WorkflowCapabilityAction[];
}) {
  const { t } = useI18n();
  return (
    <div
      className="mt-4 rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--background)] p-3"
      id={panelId}
      onClick={(event) => event.stopPropagation()}
      ref={panelRef}
      role="region"
      tabIndex={-1}
    >
      <div className="grid gap-3">
        <TextField
          label={t("workflows.storyboard.fields.title")}
          onChange={(label) => onUpdate({ label } as Partial<WorkflowIrNode>)}
          value={node.label}
        />
        <NodeSpecificFields node={node} onUpdate={onUpdate} />
      </div>

      <div className="mt-4 flex flex-wrap gap-2 border-t border-[var(--border-soft)] pt-3">
        <button
          className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-xs font-medium text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-not-allowed disabled:opacity-50"
          disabled={!canInsertAfter}
          onClick={onAddStep}
          type="button"
        >
          {t("workflows.storyboard.add_step")}
        </button>
        <button
          className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-xs font-medium text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-not-allowed disabled:opacity-50"
          disabled={!canInsertAfter}
          onClick={onAddBranch}
          type="button"
        >
          {t("workflows.storyboard.add_branch")}
        </button>
        <button
          className="ml-auto rounded-[var(--radius-sm)] border border-[var(--border-soft)] px-3 py-2 text-xs font-medium text-[var(--destructive)] transition-colors hover:bg-[var(--destructive-background)] disabled:cursor-not-allowed disabled:opacity-50"
          disabled={!canRemove}
          onClick={onRemove}
          type="button"
        >
          {t("workflows.storyboard.remove_step")}
        </button>
      </div>

      {isAddingStep && (
        <div className="mt-3 rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-3">
          <p className="text-xs font-semibold text-[var(--foreground)]">
            {t("workflows.storyboard.add_step_title")}
          </p>
          <div className="mt-2 grid gap-2">
            {stepActions.length === 0 ? (
              <p className="text-xs leading-5 text-[var(--foreground-muted)]">
                {t("workflows.storyboard.no_catalog_actions")}
              </p>
            ) : (
              stepActions.map((action) => (
                <button
                  className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--background)] px-3 py-2 text-left transition-colors hover:bg-[var(--fill-hover)]"
                  key={action.id}
                  onClick={() => onInsertAction(action)}
                  type="button"
                >
                  <span className="block text-xs font-semibold text-[var(--foreground)]">
                    {action.title}
                  </span>
                  <span className="mt-1 block text-[11px] leading-4 text-[var(--foreground-muted)]">
                    {action.outcome}
                  </span>
                </button>
              ))
            )}
          </div>
        </div>
      )}

      {isBranching && (
        <div className="mt-3 rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-3">
          <TextAreaField
            label={t("workflows.storyboard.branch_condition")}
            onChange={onBranchConditionChange}
            value={branchCondition}
          />
          <div className="mt-3 flex justify-end gap-2">
            <button
              className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] px-3 py-2 text-xs font-medium text-[var(--foreground-muted)] transition-colors hover:bg-[var(--fill-hover)]"
              onClick={onBranchCancel}
              type="button"
            >
              {t("common.cancel")}
            </button>
            <button
              className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-3 py-2 text-xs font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)]"
              onClick={onBranchCommit}
              type="button"
            >
              {t("workflows.storyboard.create_branch")}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

function NodeSpecificFields({
  node,
  onUpdate,
}: {
  node: WorkflowIrNode;
  onUpdate: (patch: Partial<WorkflowIrNode>) => void;
}) {
  const { t } = useI18n();
  const { getPermissionLabel } = useHumanTrust();

  if (node.kind === "input") {
    return null;
  }

  if (node.kind === "agent") {
    return (
      <TextAreaField
        label={t("workflows.storyboard.fields.objective")}
        onChange={(objective) => onUpdate({ objective } as Partial<WorkflowIrNode>)}
        value={node.objective}
      />
    );
  }

  if (node.kind === "conditional") {
    return (
      <TextAreaField
        label={t("workflows.storyboard.fields.condition")}
        onChange={(condition) => onUpdate({ condition } as Partial<WorkflowIrNode>)}
        value={node.condition}
      />
    );
  }

  if (node.kind === "router") {
    return (
      <>
        {node.routes.map((route, index) => (
          <TextField
            key={route.port}
            label={t("workflows.storyboard.fields.route_condition", {
              port: route.port,
            })}
            onChange={(condition) =>
              onUpdate({
                routes: node.routes.map((entry, routeIndex) =>
                  routeIndex === index ? { ...entry, condition } : entry,
                ),
              } as Partial<WorkflowIrNode>)
            }
            value={route.condition}
          />
        ))}
      </>
    );
  }

  if (node.kind === "loop") {
    return null;
  }

  if (node.kind === "permission") {
    return (
      <>
        <SelectField
          label={t("workflows.storyboard.fields.permission")}
          onChange={(permission) =>
            onUpdate({
              permission: permission as Extract<WorkflowIrNode, { kind: "permission" }>["permission"],
            } as Partial<WorkflowIrNode>)
          }
          options={[
            "file_read",
            "file_write",
            "network",
            "process",
            "mcp_tool",
            "custom",
          ].map((permission) => ({
            label: getPermissionLabel(permission),
            value: permission,
          }))}
          value={node.permission}
        />
        <TextAreaField
          label={t("workflows.storyboard.fields.reason")}
          onChange={(reason) => onUpdate({ reason } as Partial<WorkflowIrNode>)}
          value={node.reason}
        />
      </>
    );
  }

  if (node.kind === "mcp_tool") {
    return null;
  }

  if (node.kind === "system_action") {
    return null;
  }

  return null;
}

export function WorkflowDeveloperFields({
  onWorkflowIrChange,
  workflowIr,
}: {
  onWorkflowIrChange: (workflowIr: WorkflowIr) => void;
  workflowIr: WorkflowIr;
}) {
  const { t } = useI18n();
  const [error, setError] = useState<string | null>(null);
  const nodes = workflowIr.nodes.filter(hasDeveloperFields);

  function updateNode(nodeId: string, patch: Partial<WorkflowIrNode>) {
    try {
      setError(null);
      onWorkflowIrChange(updateWorkflowIrNode(workflowIr, nodeId, patch));
    } catch (updateError) {
      setError(friendlyStoryboardError(updateError, t));
    }
  }

  return (
    <section>
      <h3 className="text-xs font-semibold uppercase text-[var(--foreground-subtle)]">
        {t("workflows.inspect.workflow_plumbing")}
      </h3>
      <p className="mt-1 text-xs leading-5 text-[var(--foreground-muted)]">
        {t("workflows.inspect.workflow_plumbing_description")}
      </p>
      {error && (
        <p
          className="mt-2 rounded-[var(--radius-sm)] border border-[var(--destructive)]/30 bg-[var(--destructive-background)] p-3 text-xs leading-5 text-[var(--destructive)]"
          role="alert"
        >
          {error}
        </p>
      )}
      {nodes.length === 0 ? (
        <p className="mt-2 rounded-[var(--radius-sm)] border border-dashed border-[var(--border-strong)] bg-[var(--accent-background)] p-3 text-xs leading-5 text-[var(--foreground-muted)]">
          {t("workflows.inspect.no_workflow_plumbing")}
        </p>
      ) : (
        <div className="mt-2 space-y-2">
          {nodes.map((node) => (
            <details
              className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--accent-background)]"
              key={node.id}
            >
              <summary className="cursor-pointer px-3 py-2 text-xs font-semibold text-[var(--foreground)]">
                {node.label}
              </summary>
              <div className="grid gap-3 border-t border-[var(--border-soft)] bg-[var(--background)] p-3">
                <DeveloperNodeFields
                  node={node}
                  onUpdate={(patch) => updateNode(node.id, patch)}
                />
              </div>
            </details>
          ))}
        </div>
      )}
    </section>
  );
}

function DeveloperNodeFields({
  node,
  onUpdate,
}: {
  node: WorkflowIrNode;
  onUpdate: (patch: Partial<WorkflowIrNode>) => void;
}) {
  const { t } = useI18n();
  const { getToolKindLabel } = useHumanTrust();

  if (node.kind === "input") {
    return (
      <TextField
        label={t("workflows.storyboard.fields.input_key")}
        onChange={(outputKey) => onUpdate({ outputKey } as Partial<WorkflowIrNode>)}
        value={node.outputKey}
      />
    );
  }

  if (node.kind === "router") {
    return (
      <TextAreaField
        label={t("workflows.storyboard.fields.router_expression")}
        onChange={(expression) => onUpdate({ expression } as Partial<WorkflowIrNode>)}
        value={node.expression}
      />
    );
  }

  if (node.kind === "loop") {
    return (
      <>
        <TextField
          label={t("workflows.storyboard.fields.items_mapping")}
          onChange={(itemsMapping) => onUpdate({ itemsMapping } as Partial<WorkflowIrNode>)}
          value={node.itemsMapping}
        />
        <TextField
          label={t("workflows.storyboard.fields.item_variable")}
          onChange={(itemVariable) => onUpdate({ itemVariable } as Partial<WorkflowIrNode>)}
          value={node.itemVariable}
        />
      </>
    );
  }

  if (node.kind === "mcp_tool") {
    return (
      <>
        <TextField
          label={t("workflows.storyboard.fields.server")}
          onChange={(serverName) => onUpdate({ serverName } as Partial<WorkflowIrNode>)}
          value={node.serverName}
        />
        <TextField
          label={t("workflows.storyboard.fields.tool")}
          onChange={(toolName) => onUpdate({ toolName } as Partial<WorkflowIrNode>)}
          value={node.toolName}
        />
        <McpSchemaFields
          emptyLabel={t("workflows.storyboard.no_schema_fields")}
          inputSchema={node.inputSchema}
          onChange={(argumentsValue) =>
            onUpdate({ arguments: argumentsValue } as Partial<WorkflowIrNode>)
          }
          values={asRecord(node.arguments)}
        />
      </>
    );
  }

  if (node.kind === "system_action") {
    return (
      <>
        <SelectField
          label={t("workflows.storyboard.fields.action_type")}
          onChange={(actionType) =>
            onUpdate({
              actionType: actionType as Extract<WorkflowIrNode, { kind: "system_action" }>["actionType"],
            } as Partial<WorkflowIrNode>)
          }
          options={["shell", "python", "binary"].map((actionType) => ({
            label: getToolKindLabel(actionType),
            value: actionType,
          }))}
          value={node.actionType}
        />
        <TextField
          label={t("workflows.storyboard.fields.command")}
          onChange={(command) => onUpdate({ command } as Partial<WorkflowIrNode>)}
          value={node.command}
        />
        <TextField
          label={t("workflows.storyboard.fields.arguments")}
          onChange={(value) =>
            onUpdate({
              args: value.split(/\s+/).filter(Boolean),
            } as Partial<WorkflowIrNode>)
          }
          value={node.args.join(" ")}
        />
      </>
    );
  }

  if (node.kind === "output") {
    return (
      <TextField
        label={t("workflows.storyboard.fields.output_mapping")}
        onChange={(inputMapping) => onUpdate({ inputMapping } as Partial<WorkflowIrNode>)}
        value={node.inputMapping}
      />
    );
  }

  return null;
}

function hasDeveloperFields(node: WorkflowIrNode) {
  return (
    node.kind === "input" ||
    node.kind === "router" ||
    node.kind === "loop" ||
    node.kind === "mcp_tool" ||
    node.kind === "system_action" ||
    node.kind === "output"
  );
}

function TextField({
  label,
  onChange,
  value,
}: {
  label: string;
  onChange: (value: string) => void;
  value: string;
}) {
  return (
    <label className="flex flex-col gap-1.5">
      <span className="text-xs font-medium text-[var(--foreground-muted)]">
        {label}
      </span>
      <input
        className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm text-[var(--foreground)] outline-none transition-colors placeholder:text-[var(--foreground-subtle)] focus:bg-[var(--accent-background)]"
        onChange={(event) => onChange(event.target.value)}
        value={value}
      />
    </label>
  );
}

function TextAreaField({
  label,
  onChange,
  value,
}: {
  label: string;
  onChange: (value: string) => void;
  value: string;
}) {
  return (
    <label className="flex flex-col gap-1.5">
      <span className="text-xs font-medium text-[var(--foreground-muted)]">
        {label}
      </span>
      <textarea
        className="min-h-24 resize-y rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm leading-5 text-[var(--foreground)] outline-none transition-colors placeholder:text-[var(--foreground-subtle)] focus:bg-[var(--accent-background)]"
        onChange={(event) => onChange(event.target.value)}
        value={value}
      />
    </label>
  );
}

function SelectField({
  label,
  onChange,
  options,
  value,
}: {
  label: string;
  onChange: (value: string) => void;
  options: SelectFieldOption[];
  value: string;
}) {
  return (
    <label className="flex flex-col gap-1.5">
      <span className="text-xs font-medium text-[var(--foreground-muted)]">
        {label}
      </span>
      <select
        className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm text-[var(--foreground)] outline-none transition-colors focus:bg-[var(--accent-background)]"
        onChange={(event) => onChange(event.target.value)}
        value={value}
      >
        {options.map((option) => {
          const optionValue = typeof option === "string" ? option : option.value;
          const optionLabel = typeof option === "string" ? option : option.label;

          return (
            <option key={optionValue} value={optionValue}>
              {optionLabel}
            </option>
          );
        })}
      </select>
    </label>
  );
}

export function buildWorkflowStoryboardModel(
  workflowIr: WorkflowIr,
  t: TranslateFn,
): StoryboardItem[] {
  const nodeById = new Map(workflowIr.nodes.map((node) => [node.id, node]));
  const edgesBySource = new Map<string, WorkflowIrEdge[]>();
  for (const edge of workflowIr.edges) {
    edgesBySource.set(edge.sourceNodeId, [
      ...(edgesBySource.get(edge.sourceNodeId) ?? []),
      edge,
    ]);
  }

  return topologicalWorkflowNodes(workflowIr).map((node, index) => {
    const nature = storyboardNatureForNode(node);
    return {
      branches: branchEdgesForNode(node, edgesBySource.get(node.id) ?? [], nodeById, t),
      detail: storyboardDetailForNode(node, t),
      id: node.id,
      index: index + 1,
      nature,
      natureLabel: t(`workflows.storyboard.natures.${nature}`),
      node,
      title: storyboardTitleForNode(node, t),
    };
  });
}

function topologicalWorkflowNodes(workflowIr: WorkflowIr) {
  const originalIndex = new Map(workflowIr.nodes.map((node, index) => [node.id, index]));
  const nodeById = new Map(workflowIr.nodes.map((node) => [node.id, node]));
  const outgoing = new Map<string, string[]>();
  const indegree = new Map(workflowIr.nodes.map((node) => [node.id, 0]));

  workflowIr.edges.forEach((edge) => {
    if (!nodeById.has(edge.sourceNodeId) || !nodeById.has(edge.targetNodeId)) {
      return;
    }
    outgoing.set(edge.sourceNodeId, [
      ...(outgoing.get(edge.sourceNodeId) ?? []),
      edge.targetNodeId,
    ]);
    indegree.set(edge.targetNodeId, (indegree.get(edge.targetNodeId) ?? 0) + 1);
  });

  const queue = [...indegree]
    .filter(([, degree]) => degree === 0)
    .map(([nodeId]) => nodeId)
    .sort((left, right) => (originalIndex.get(left) ?? 0) - (originalIndex.get(right) ?? 0));
  const ordered: WorkflowIrNode[] = [];
  const visited = new Set<string>();

  for (let cursor = 0; cursor < queue.length; cursor += 1) {
    const nodeId = queue[cursor];
    if (visited.has(nodeId)) {
      continue;
    }
    visited.add(nodeId);
    const node = nodeById.get(nodeId);
    if (node) {
      ordered.push(node);
    }

    const next = [...(outgoing.get(nodeId) ?? [])].sort(
      (left, right) => (originalIndex.get(left) ?? 0) - (originalIndex.get(right) ?? 0),
    );
    next.forEach((targetId) => {
      const degree = (indegree.get(targetId) ?? 0) - 1;
      indegree.set(targetId, degree);
      if (degree === 0) {
        queue.push(targetId);
      }
    });
  }

  workflowIr.nodes.forEach((node) => {
    if (!visited.has(node.id)) {
      ordered.push(node);
    }
  });

  return ordered;
}

function branchEdgesForNode(
  node: WorkflowIrNode,
  edges: WorkflowIrEdge[],
  nodeById: Map<string, WorkflowIrNode>,
  t: TranslateFn,
) {
  const shouldShowBranches =
    node.kind === "router" ||
    node.kind === "conditional" ||
    node.kind === "loop" ||
    node.kind === "permission" ||
    edges.length > 1;

  if (!shouldShowBranches) {
    return [];
  }

  return edges.map((edge) => {
    const target = nodeById.get(edge.targetNodeId);
    return {
      id: edge.id,
      label: branchLabel(node, edge.sourcePort, t),
      port: edge.sourcePort,
      targetId: edge.targetNodeId,
      targetLabel: target ? storyboardTitleForNode(target, t) : edge.targetNodeId,
    };
  });
}

function branchLabel(node: WorkflowIrNode, port: string, t: TranslateFn) {
  if (node.kind === "conditional") {
    return port === "true"
      ? t("workflows.storyboard.branch_true")
      : t("workflows.storyboard.branch_false");
  }

  if (node.kind === "router") {
    const route = node.routes.find((entry) => entry.port === port);
    return route
      ? t("workflows.storyboard.branch_when", { condition: route.condition })
      : t("workflows.storyboard.branch_port", { port });
  }

  if (node.kind === "loop") {
    return port === "item"
      ? t("workflows.storyboard.branch_item")
      : t("workflows.storyboard.branch_done");
  }

  if (node.kind === "permission") {
    if (port === "approved") {
      return t("workflows.storyboard.branch_approved");
    }
    if (port === "denied") {
      return t("workflows.storyboard.branch_denied");
    }
  }

  return t("workflows.storyboard.branch_port", { port });
}

function storyboardNatureForNode(node: WorkflowIrNode): StoryboardNature {
  if (node.kind === "permission") {
    return "approve";
  }

  if (node.kind === "agent" || node.kind === "router" || node.kind === "conditional" || node.kind === "loop") {
    return "think";
  }

  if (node.kind === "mcp_tool") {
    return /^(read|get|list|search|fetch|scan|query)/i.test(node.toolName)
      ? "read"
      : "act";
  }

  if (node.kind === "input") {
    return "read";
  }

  return "act";
}

function storyboardDetailForNode(node: WorkflowIrNode, t: TranslateFn) {
  if (node.kind === "input") {
    return t("workflows.storyboard.details.input");
  }

  if (node.kind === "agent") {
    return agentStoryboardCopy(node, t).detail;
  }

  if (node.kind === "router") {
    return t("workflows.storyboard.details.router");
  }

  if (node.kind === "conditional") {
    return t("workflows.storyboard.details.conditional", {
      condition: firstSentenceForWorkflowPreview(node.condition),
    });
  }

  if (node.kind === "loop") {
    return t("workflows.storyboard.details.loop", {
      items: node.itemsMapping,
      variable: node.itemVariable,
    });
  }

  if (node.kind === "permission") {
    return t("workflows.storyboard.details.permission", {
      reason: firstSentenceForWorkflowPreview(node.reason),
    });
  }

  if (node.kind === "mcp_tool") {
    return mcpStoryboardCopy(node.serverName, node.toolName, t).detail;
  }

  if (node.kind === "system_action") {
    return node.command === "open"
      ? t("workflows.storyboard.details.system_open")
      : t("workflows.storyboard.details.system_action");
  }

  return t("workflows.storyboard.details.output");
}

function storyboardTitleForNode(node: WorkflowIrNode, t: TranslateFn) {
  if (node.kind === "input") {
    return t("workflows.storyboard.titles.input");
  }
  if (node.kind === "output") {
    return t("workflows.storyboard.titles.output");
  }
  if (node.kind === "agent") {
    return agentStoryboardCopy(node, t).title;
  }
  if (node.kind === "permission") {
    return t("workflows.storyboard.titles.permission");
  }
  if (node.kind === "mcp_tool") {
    return mcpStoryboardCopy(node.serverName, node.toolName, t).title;
  }
  if (node.kind === "system_action") {
    return node.command === "open"
      ? t("workflows.storyboard.titles.system_open")
      : t("workflows.storyboard.titles.system_action");
  }
  if (node.kind === "conditional" || node.kind === "router") {
    return t("workflows.storyboard.titles.decision");
  }
  if (node.kind === "loop") {
    return t("workflows.storyboard.titles.loop");
  }
  return t("workflows.storyboard.titles.output");
}

function agentStoryboardCopy(
  node: Extract<WorkflowIrNode, { kind: "agent" }>,
  t: TranslateFn,
) {
  const text = `${node.label} ${node.objective}`.toLowerCase();
  if (/(draft|reply|email|message)/.test(text)) {
    return {
      title: t("workflows.storyboard.titles.agent_draft"),
      detail: t("workflows.storyboard.details.agent_draft"),
    };
  }
  if (/(decide|recommend|priority|risk|route|branch)/.test(text)) {
    return {
      title: t("workflows.storyboard.titles.agent_decide"),
      detail: t("workflows.storyboard.details.agent_decide"),
    };
  }
  return {
    title: t("workflows.storyboard.titles.agent_summarize"),
    detail: t("workflows.storyboard.details.agent_summarize"),
  };
}

function mcpStoryboardCopy(serverName: string, toolName: string, t: TranslateFn) {
  const key = `${serverName}.${toolName}`;
  const known = {
    "local_filesystem.list_directory": "list_directory",
    "local_filesystem.read_file": "read_file",
    "local_filesystem.write_file": "write_file",
    "macos_applescript.read_system_calendar": "read_calendar",
    "macos_applescript.trigger_system_notification": "notify",
    "macos_applescript.draft_system_email": "draft_email",
    "macos_applescript.read_system_emails": "read_mail",
    "macos_applescript.read_system_reminders": "read_reminders",
    "taskflow_native.folder_read": "folder_read",
    "taskflow_native.write_markdown_report": "write_report",
    "taskflow_native.preview_report": "preview_report",
  }[key];
  if (known) {
    return {
      title: t(`workflows.storyboard.titles.${known}`),
      detail: t(`workflows.storyboard.details.${known}`),
    };
  }
  return {
    title: t("workflows.storyboard.titles.connected_action", {
      action: humanize(toolName),
    }),
    detail: t("workflows.storyboard.details.connected_action", {
      action: humanize(toolName),
      app: humanize(serverName),
    }),
  };
}

function humanize(value: string) {
  return value
    .replace(/[_-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .replace(/\b\w/g, (character) => character.toUpperCase());
}

function friendlyStoryboardError(error: unknown, t: TranslateFn) {
  const message = extractErrorMessage(error);
  const normalized = message.toLowerCase();

  if (normalized.includes("unknown source node")) {
    return t("workflows.storyboard.errors.unknown_step");
  }
  if (normalized.includes("one clear outgoing path")) {
    return t("workflows.storyboard.errors.single_path");
  }
  if (normalized.includes("cannot be inserted")) {
    return t("workflows.storyboard.errors.unsupported_action");
  }
  if (normalized.includes("cannot be removed")) {
    return t("workflows.storyboard.errors.protected_step");
  }
  if (normalized.includes("one outgoing path")) {
    return t("workflows.storyboard.errors.branching_remove");
  }
  if (
    !message ||
    normalized.includes("zoderror") ||
    normalized.includes("invalid_type") ||
    normalized.includes("duplicate") ||
    normalized.includes("requires")
  ) {
    return t("workflows.storyboard.errors.validation");
  }

  return t("workflows.storyboard.errors.generic", { error: message });
}

function extractErrorMessage(error: unknown) {
  if (typeof error === "string") {
    return error.trim();
  }

  if (error && typeof error === "object") {
    const record = error as Record<string, unknown>;
    if (typeof record.message === "string" && record.message.trim()) {
      return record.message.trim();
    }
  }

  return "";
}

export function WorkflowNatureIcon({ nature }: { nature: StoryboardNature }) {
  if (nature === "read") {
    return (
      <svg aria-hidden="true" className="h-3 w-3" fill="none" stroke="currentColor" strokeWidth="2" viewBox="0 0 24 24">
        <path d="M4 5h16" />
        <path d="M4 12h16" />
        <path d="M4 19h10" />
      </svg>
    );
  }

  if (nature === "think") {
    return (
      <svg aria-hidden="true" className="h-3 w-3" fill="none" stroke="currentColor" strokeWidth="2" viewBox="0 0 24 24">
        <path d="M12 4a7 7 0 0 0-4 12.7V20h8v-3.3A7 7 0 0 0 12 4Z" />
        <path d="M9 20h6" />
      </svg>
    );
  }

  if (nature === "approve") {
    return (
      <svg aria-hidden="true" className="h-3 w-3" fill="none" stroke="currentColor" strokeWidth="2" viewBox="0 0 24 24">
        <path d="M12 3 5 6v6c0 4 2.7 7.5 7 9 4.3-1.5 7-5 7-9V6l-7-3Z" />
        <path d="m9 12 2 2 4-5" />
      </svg>
    );
  }

  return (
    <svg aria-hidden="true" className="h-3 w-3" fill="none" stroke="currentColor" strokeWidth="2" viewBox="0 0 24 24">
      <path d="m5 12 5 5L20 7" />
    </svg>
  );
}
