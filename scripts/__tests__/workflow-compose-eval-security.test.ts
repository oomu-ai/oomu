import { describe, expect, it } from "vitest";
import {
  isContractValidResponse,
  readBoundedResponseBody,
  validateHarnessEndpoint,
} from "../workflow-compose-eval.mjs";

describe("workflow compose evaluation harness boundary", () => {
  it("accepts HTTPS or exact numeric loopback with an explicit port only", () => {
    expect(validateHarnessEndpoint("https://eval.example.test/native").protocol).toBe("https:");
    expect(validateHarnessEndpoint("http://127.0.0.1:43123/native").hostname).toBe("127.0.0.1");
    expect(validateHarnessEndpoint("http://[::1]:43123/native").port).toBe("43123");

    for (const endpoint of [
      "http://localhost:43123/native",
      "http://127.0.0.2:43123/native",
      "http://127.0.0.1/native",
      "http://127.0.0.1:0/native",
      "https://user:password@eval.example.test/native",
      "https://eval.example.test/native?token=canary",
      "https://eval.example.test/native#secret",
    ]) {
      expect(() => validateHarnessEndpoint(endpoint)).toThrow(/endpoint/i);
    }
  });

  it("cancels an oversized chunked response before buffering the remainder", async () => {
    let cancelled = false;
    let chunk = 0;
    const response = new Response(new ReadableStream<Uint8Array>({
      pull(controller) {
        chunk += 1;
        controller.enqueue(new Uint8Array(5).fill(chunk));
        if (chunk === 4) controller.close();
      },
      cancel() {
        cancelled = true;
      },
    }));

    await expect(readBoundedResponseBody(response, 8)).rejects.toThrow(
      "workflow_eval_harness_response_oversized",
    );
    expect(cancelled).toBe(true);
    expect(chunk).toBeLessThan(4);
  });

  it("does not redact legitimate token-cost fields before contract validation", () => {
    const response = {
      status: "composed",
      workflowIr: {
        schemaVersion: "1.0.0",
        workflowId: "token-budget-check",
        name: "Token budget check",
        tokenCost: 512,
        nodes: [
          { id: "input", kind: "input" },
          { id: "output", kind: "output" },
        ],
        edges: [],
      },
    };
    expect(isContractValidResponse(response, { actions: [] })).toBe(true);
    expect(response.workflowIr.tokenCost).toBe(512);
  });
});
