"use client";

import { useI18n } from "@/context/I18nContext";

type McpSchemaFieldDefinition = {
  description?: string;
  enumValues?: string[];
  maximum?: number;
  minimum?: number;
  name: string;
  required: boolean;
  title?: string;
  type: "string" | "number" | "integer" | "boolean";
};

type McpSchemaFieldsProps = {
  emptyLabel?: string;
  inputSchema: unknown;
  onChange: (value: Record<string, unknown>) => void;
  values: Record<string, unknown>;
};

export function McpSchemaFields({
  emptyLabel,
  inputSchema,
  onChange,
  values,
}: McpSchemaFieldsProps) {
  const { t } = useI18n();
  const fields = schemaFields(inputSchema);

  if (fields.length === 0) {
    return (
      <div className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--background)] p-3 text-xs text-[var(--foreground-muted)]">
        {emptyLabel ?? t("workflows.storyboard.no_schema_fields")}
      </div>
    );
  }

  return (
    <div className="grid gap-3">
      {fields.map((field) => (
        <McpSchemaField
          field={field}
          key={field.name}
          onChange={(value) => onChange({ ...values, [field.name]: value })}
          value={values[field.name] ?? defaultValueForSchemaField(field)}
        />
      ))}
    </div>
  );
}

function schemaFields(inputSchema: unknown): McpSchemaFieldDefinition[] {
  const schema = asRecord(inputSchema);
  const properties = asRecord(schema.properties);
  const required = Array.isArray(schema.required)
    ? schema.required.filter((item): item is string => typeof item === "string")
    : [];

  return Object.entries(properties).flatMap(([name, rawDefinition]) => {
    const definition = asRecord(rawDefinition);
    const type = normalizeSchemaType(definition.type);

    if (!type) {
      return [];
    }

    return [
      {
        description:
          typeof definition.description === "string"
            ? definition.description
            : undefined,
        enumValues: Array.isArray(definition.enum)
          ? definition.enum.map(String)
          : undefined,
        maximum:
          typeof definition.maximum === "number" ? definition.maximum : undefined,
        minimum:
          typeof definition.minimum === "number" ? definition.minimum : undefined,
        name,
        required: required.includes(name),
        title: typeof definition.title === "string" ? definition.title : undefined,
        type,
      },
    ];
  });
}

export function defaultArgumentsForSchema(inputSchema: unknown) {
  return Object.fromEntries(
    schemaFields(inputSchema).map((field) => [
      field.name,
      defaultValueForSchemaField(field),
    ]),
  );
}

function defaultValueForSchemaField(field: McpSchemaFieldDefinition) {
  if (field.enumValues?.[0]) {
    return field.enumValues[0];
  }

  if (field.type === "boolean") {
    return false;
  }

  if (field.type === "number" || field.type === "integer") {
    return field.minimum ?? 0;
  }

  return "";
}

function McpSchemaField({
  field,
  onChange,
  value,
}: {
  field: McpSchemaFieldDefinition;
  onChange: (value: unknown) => void;
  value: unknown;
}) {
  const label = `${field.title ?? field.name}${field.required ? " *" : ""}`;
  const textValue =
    typeof value === "string" || typeof value === "number" ? String(value) : "";
  const fieldClass =
    "rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm text-[var(--foreground)] outline-none transition-colors focus:bg-[var(--accent-background)]";

  if (field.enumValues && field.enumValues.length > 0) {
    return (
      <label className="flex flex-col gap-1.5">
        <span className="text-xs font-medium text-[var(--foreground-muted)]">
          {label}
        </span>
        <select
          className={fieldClass}
          onChange={(event) => onChange(event.target.value)}
          value={String(value ?? field.enumValues[0] ?? "")}
        >
          {field.enumValues.map((option) => (
            <option key={option} value={option}>
              {option}
            </option>
          ))}
        </select>
        <FieldHint field={field} />
      </label>
    );
  }

  if (field.type === "boolean") {
    return (
      <label className="flex items-center gap-3 rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2.5">
        <input
          checked={Boolean(value)}
          className="h-4 w-4 accent-[var(--accent)]"
          onChange={(event) => onChange(event.target.checked)}
          type="checkbox"
        />
        <span className="text-sm text-[var(--foreground)]">{label}</span>
      </label>
    );
  }

  if (field.type === "number" || field.type === "integer") {
    return (
      <label className="flex flex-col gap-1.5">
        <span className="text-xs font-medium text-[var(--foreground-muted)]">
          {label}
        </span>
        <input
          className={fieldClass}
          max={field.maximum}
          min={field.minimum}
          onChange={(event) => {
            const next = event.target.value;
            onChange(next === "" ? "" : Number(next));
          }}
          step={field.type === "integer" ? 1 : "any"}
          type="number"
          value={textValue}
        />
        <FieldHint field={field} />
      </label>
    );
  }

  return (
    <label className="flex flex-col gap-1.5">
      <span className="text-xs font-medium text-[var(--foreground-muted)]">
        {label}
      </span>
      <input
        className={fieldClass}
        onChange={(event) => onChange(event.target.value)}
        type="text"
        value={textValue}
      />
      <FieldHint field={field} />
    </label>
  );
}

function FieldHint({ field }: { field: McpSchemaFieldDefinition }) {
  const { t } = useI18n();
  const constraints = [
    field.description,
    field.minimum !== undefined
      ? t("workflows.storyboard.schema_min", { value: field.minimum })
      : null,
    field.maximum !== undefined
      ? t("workflows.storyboard.schema_max", { value: field.maximum })
      : null,
  ]
    .filter(Boolean)
    .join(" / ");

  if (!constraints) {
    return null;
  }

  return (
    <span className="text-[11px] leading-4 text-[var(--foreground-subtle)]">
      {constraints}
    </span>
  );
}

function normalizeSchemaType(value: unknown): McpSchemaFieldDefinition["type"] | null {
  const type = Array.isArray(value) ? value.find((item) => item !== "null") : value;

  if (
    type === "string" ||
    type === "number" ||
    type === "integer" ||
    type === "boolean"
  ) {
    return type;
  }

  return null;
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}
