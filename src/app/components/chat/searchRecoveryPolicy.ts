export function runWeb(recovery: boolean, resume?: { turnState?: string }) {
  return !recovery || resume?.turnState === "interrupted";
}
