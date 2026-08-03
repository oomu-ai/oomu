import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { inspectRuntimeText } from "../check-real-components.mjs";

describe("real-component architecture gate", () => {
  it("can scan its own implementation after it is tracked", () => {
    const path = "scripts/check-real-components.mjs";
    expect(inspectRuntimeText(path, readFileSync(resolve(process.cwd(), path), "utf8"))).toEqual([]);
  });

  it("fails a newly introduced status-only worker", () => {
    const source = `fn spawn_${"placeholder"}_worker() { status = "running"; }`;
    expect(inspectRuntimeText("src-tauri/src/channel.rs", source)).toHaveLength(1);
  });

  it("fails a newly introduced workflow substitute generator", () => {
    const source = `fn ${"deterministic"}_compose_fallback() {}`;
    expect(inspectRuntimeText("src-tauri/src/compiler.rs", source)).toHaveLength(1);
  });

  it("fails manufactured authorization success and estimated hardware evidence", () => {
    const authorization = `AuthorizedActions::${"Hallucinated"}Success`;
    const hardware = `let profile = ${"compute"}_score(${"estimated"}_vram_gb);`;
    expect(inspectRuntimeText("src-tauri/src/shield.rs", authorization)).toHaveLength(1);
    expect(inspectRuntimeText("src-tauri/src/sys_info.rs", hardware)).toHaveLength(2);
  });

  it("does not misclassify the real external provider brand", () => {
    const source = `const ENDPOINT: &str = "https://api.synthetic.new";`;
    expect(inspectRuntimeText("src-tauri/src/inference/provider.rs", source)).toEqual([]);
  });

  it("does not reject bounded real-component retry policy", () => {
    const source = `for attempt in 0..2 { return invoke_real_provider(attempt); }`;
    expect(inspectRuntimeText("src-tauri/src/inference/provider.rs", source)).toEqual([]);
  });

  it("rejects invented workflow inputs and fallback success copy", () => {
    const path = `workspace/vwa-${"output"}.txt`;
    const content = `OOMU VWA verified ${"file write"}`;
    const fallback = `const fallback${"Success"} = "Wrote file";`;
    expect(inspectRuntimeText("src-tauri/src/agentic_loop.rs", `${path}\n${content}`)).toHaveLength(2);
    expect(inspectRuntimeText("src/app/components/ChatScreen.tsx", fallback)).toHaveLength(1);
  });

  it("rejects tool assertions without bindings and false conversational continuation", () => {
    const toolAssertion = `You must immediately call the corresponding ${"tool"}.`;
    const continuation = `Continuing with conversational ${"response"}.`;
    expect(inspectRuntimeText("src-tauri/src/agent_manager.rs", toolAssertion)).toHaveLength(1);
    expect(inspectRuntimeText("src-tauri/src/agentic_loop.rs", continuation)).toHaveLength(1);
  });

  it("rejects static capability substitution and fabricated diagnostic identity", () => {
    const staticCatalog = `function buildStaticWorkflow${"CapabilityCatalog"}() {}`;
    const diagnosticIdentity = `value.unwrap_or_else(|| "agent-${"oomu"}".to_string())`;
    expect(inspectRuntimeText("src/app/components/workflowCapabilityCatalog.ts", staticCatalog)).toHaveLength(1);
    expect(inspectRuntimeText("src-tauri/src/system_diagnostics.rs", diagnosticIdentity)).toHaveLength(1);
  });

  it("rejects runtime fallback wrappers and unverified-action passthrough", () => {
    const fallbackWrapper = `export async function ${"safe"}Invoke() {}`;
    const passthrough = `message.${"simulated"}ToolStatement = true;`;
    expect(inspectRuntimeText("src/lib/safeInvoke.ts", fallbackWrapper)).toHaveLength(1);
    expect(inspectRuntimeText("src/app/components/ChatScreen.tsx", passthrough)).toHaveLength(1);
  });

  it("rejects AppleScript failures disguised as successful tool results", () => {
    const collectionFailure = `def degraded_collection_${"result"}(name, metadata, error):\n    return text_${"result"}("[]")`;
    const permissionFailure = `def permission_blocked_or_timed_out_${"result"}():\n    return text_${"result"}("blocked")`;
    expect(inspectRuntimeText("src-tauri/resources/mcp/mcp_applescript.py", collectionFailure)).toHaveLength(1);
    expect(inspectRuntimeText("src-tauri/resources/mcp/mcp_applescript.py", permissionFailure)).toHaveLength(1);
  });
});
