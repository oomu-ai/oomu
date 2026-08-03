export type PrimaryAppSection =
  | "chat"
  | "projects"
  | "tasks"
  | "artifacts"
  | "connections"
  | "mods";

type RailLessAppSection = "agents" | "hero";
type UtilityAppSection = "ledger" | "settings" | "user_config" | "developer";

export type ResolvedAppSection =
  | PrimaryAppSection
  | RailLessAppSection
  | UtilityAppSection;
