import { describe, expect, it } from "vitest";
import {
  DEFAULT_LOCAL_MODEL_ID,
  REMOTE_MODEL_CATALOG,
  SYSTEM_MODEL_TEMPLATES,
  contextLabelForModel,
  defaultReasoningLevelForProvider,
  resolveReasoningFallback,
  supportedReasoningLevelsForModel,
} from "../modelRegistry";

describe("modelRegistry", () => {
  it("uses binary reasoning modes for Gemma 4 models", () => {
    expect(supportedReasoningLevelsForModel("local_model", "gemma-4-e2b")).toEqual([
      "off",
      "on",
    ]);
    expect(supportedReasoningLevelsForModel("custom", "Gemma 4 E2B")).toEqual([
      "off",
      "on",
    ]);
    expect(
      supportedReasoningLevelsForModel("local_model", "gemma-4-E4B-it-qat-q4_0-gguf"),
    ).toEqual(["off", "on"]);
  });

  it("uses the supplied 2026 catalog as the complete remote registry", () => {
    expect(DEFAULT_LOCAL_MODEL_ID).toBe("gemma-4-E2B-it-qat-q4_0-gguf");
    expect(SYSTEM_MODEL_TEMPLATES.slice(0, 3).map((template) => template.modelId)).toEqual([
      "gemma-4-E2B-it-qat-q4_0-gguf",
      "gemma-4-E4B-it-qat-q4_0-gguf",
      "gemma-4-12B-it-q8_0-gguf",
    ]);
    expect(SYSTEM_MODEL_TEMPLATES.slice(3).map((template) => template.modelId)).toEqual(
      REMOTE_MODEL_CATALOG.providers.flatMap((provider) =>
        provider.models.map((model) => model.modelId),
      ),
    );
    expect(REMOTE_MODEL_CATALOG.version).toBe("2026.08.07");
    expect(REMOTE_MODEL_CATALOG.providers.map((provider) => provider.providerId)).toEqual([
      "google",
      "openai",
      "anthropic",
      "x-ai",
      "deepseek",
      "qwen_us",
      "qwen",
      "zai",
      "zai_coding",
      "zhipu",
      "moonshot_global",
      "moonshot",
      "openrouter",
      "synthetic",
    ]);
    expect(REMOTE_MODEL_CATALOG.providers.map((provider) => provider.baseUrl)).toEqual([
      "https://generativelanguage.googleapis.com/v1beta",
      "https://api.openai.com/v1",
      "https://api.anthropic.com/v1",
      "https://api.x.ai/v1",
      "https://api.deepseek.com/v1",
      "https://dashscope-us.aliyuncs.com/compatible-mode/v1",
      "https://dashscope.aliyuncs.com/compatible-mode/v1",
      "https://api.z.ai/api/paas/v4",
      "https://api.z.ai/api/coding/paas/v4",
      "https://open.bigmodel.cn/api/paas/v4",
      "https://api.moonshot.ai/v1",
      "https://api.moonshot.cn/v1",
      "https://openrouter.ai/api/v1",
      "https://api.synthetic.ai/v1",
    ]);
    expect(SYSTEM_MODEL_TEMPLATES).toHaveLength(88);
  });

  it("uses template metadata for context and reasoning support", () => {
    expect(contextLabelForModel("google", "gemini-3.6-flash")).toBe("1M");
    expect(contextLabelForModel("openai", "gpt-5.6-sol")).toBe("1M");
    expect(contextLabelForModel("anthropic", "claude-fable-5")).toBe("1M");
    expect(contextLabelForModel("zai", "zai/glm-5.2")).toBe("1M");
    expect(supportedReasoningLevelsForModel("google", "gemini-3.6-flash")).toEqual([
      "off",
      "low",
      "medium",
      "high",
      "max",
    ]);
    expect(supportedReasoningLevelsForModel("openai", "gpt-5.6-sol")).toEqual([
      "off",
      "low",
      "medium",
      "high",
      "max",
    ]);
    expect(supportedReasoningLevelsForModel("anthropic", "claude-fable-5")).toEqual([
      "low",
      "medium",
      "high",
      "max",
    ]);
  });

  it("normalizes reasoning fallback aliases for local and max modes", () => {
    expect(resolveReasoningFallback("medium", ["off", "on"])).toBe("on");
    expect(resolveReasoningFallback("ultra", ["off", "low", "medium", "high", "max"])).toBe(
      "max",
    );
  });

  it("uses provider-specific default reasoning levels", () => {
    expect(defaultReasoningLevelForProvider("google")).toBe("medium");
    expect(defaultReasoningLevelForProvider("gemini")).toBe("medium");
    expect(defaultReasoningLevelForProvider("openai")).toBe("high");
    expect(defaultReasoningLevelForProvider("anthropic")).toBe("high");
    expect(defaultReasoningLevelForProvider("local_model")).toBe("low");
  });
});

describe("OpenRouter model catalog", () => {
  it("exposes the verified expansion with gateway reasoning controls", () => {
    const openRouter = REMOTE_MODEL_CATALOG.providers.find(
      (provider) => provider.providerId === "openrouter",
    );
    expect(openRouter?.models.slice(-8).map((model) => model.modelId)).toEqual([
      "deepseek/deepseek-v4-flash-0731",
      "deepseek/deepseek-v4-flash",
      "tencent/hy3",
      "xiaomi/mimo-v2.5",
      "z-ai/glm-5.2",
      "deepseek/deepseek-v4-pro",
      "nvidia/nemotron-3-ultra-550b-a55b:free",
      "moonshotai/kimi-k3",
    ]);
    expect(
      openRouter?.models.find((model) => model.modelId === "deepseek/deepseek-v4-pro")
        ?.thinkingSupport,
    ).toMatchObject({
      type: "reasoning_effort",
      parameterName: "reasoning.effort",
      levels: ["off", "high", "xhigh"],
      defaultLevel: "high",
    });
    expect(
      openRouter?.models.find(
        (model) => model.modelId === "nvidia/nemotron-3-ultra-550b-a55b:free",
      )?.pricingPer1M,
    ).toEqual({ input: 0, output: 0 });
  });
});
