import type { SlashCommandOption } from "../ChatComposer";
import { firstSlashTrigger } from "./browserRouting";

export function slashCommandForMessage(
  commands: SlashCommandOption[],
  message: string,
) {
  const trigger = firstSlashTrigger(message);
  if (!trigger) return null;
  return commands.find(
    (command) => command.trigger.toLowerCase() === trigger.toLowerCase(),
  ) ?? null;
}
