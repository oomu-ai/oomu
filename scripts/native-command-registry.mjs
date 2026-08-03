import { readFileSync } from "node:fs";
import path from "node:path";

export const NATIVE_COMMAND_REGISTRY_PATH = "src-tauri/src/command_registration.rs";
export const NATIVE_COMMAND_HANDLER_INVOCATION =
  "command_registration::oomu_command_handler!()";

const REGISTRY_MARKER = "tauri::generate_handler![";

export function parseNativeCommandRegistry(source) {
  const start = source.indexOf(REGISTRY_MARKER);
  if (start < 0) throw new Error("native production command registry not found");
  const bodyStart = start + REGISTRY_MARKER.length;
  const bodyEnd = source.indexOf("]", bodyStart);
  if (bodyEnd < 0) throw new Error("native production command registry is unterminated");
  return new Set(
    source
      .slice(bodyStart, bodyEnd)
      .split(",")
      .map((entry) => entry.trim())
      .filter(Boolean)
      .map((entry) => entry.split("::").at(-1)),
  );
}

export function registeredProductionCommands(root) {
  return parseNativeCommandRegistry(
    readFileSync(path.join(root, NATIVE_COMMAND_REGISTRY_PATH), "utf8"),
  );
}

export function assertNativeCommandHandlerWiring(root) {
  const startup = readFileSync(path.join(root, "src-tauri/src/lib.rs"), "utf8");
  if (!startup.includes(NATIVE_COMMAND_HANDLER_INVOCATION)) {
    throw new Error("native production command registry is not wired into app startup");
  }
}
