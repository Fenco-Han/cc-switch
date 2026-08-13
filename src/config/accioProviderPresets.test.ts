import { describe, expect, it } from "vitest";

import {
  accioProviderPresets,
  ACCIO_DEFAULT_API_FORMAT,
  ACCIO_DEEPSEEK_MODELS,
} from "./accioProviderPresets";

describe("Accio provider presets", () => {
  it("ships DeepSeek plus six multi-provider presets", () => {
    expect(accioProviderPresets).toHaveLength(7);
    expect(accioProviderPresets[0]).toMatchObject({
      name: "DeepSeek",
      category: "cn_official",
      icon: "deepseek",
      settingsConfig: {
        name: "deepseek",
        base_url: "https://api.deepseek.com",
        api_key: "",
        api_format: "openai_chat",
        model: "deepseek-chat",
        models: ACCIO_DEEPSEEK_MODELS,
      },
    });
    expect(accioProviderPresets.map((preset) => preset.settingsConfig.name)).toEqual([
      "deepseek",
      "xiaomi_mimo",
      "kimi",
      "minimax",
      "zhipu_glm",
      "nvidia",
      "openrouter",
    ]);
  });

  it("ships OpenAI-compatible presets with per-provider model lists", () => {
    const byName = Object.fromEntries(
      accioProviderPresets.map((preset) => [
        preset.settingsConfig.name,
        preset,
      ]),
    );

    for (const preset of accioProviderPresets) {
      expect(preset.settingsConfig.api_format).toBe("openai_chat");
      expect(preset.settingsConfig.models?.length).toBeGreaterThan(0);
    }

    expect(byName.kimi.settingsConfig.models?.map((model) => model.id)).toEqual([
      "kimi-k2.7-code",
      "kimi-k3",
    ]);
    expect(byName.xiaomi_mimo.settingsConfig.models?.map((model) => model.id)).toEqual([
      "mimo-v2.5-pro",
      "mimo-v2.5",
    ]);
    expect(byName.openrouter.settingsConfig.models).toHaveLength(5);
    expect(byName.nvidia.settingsConfig.base_url).toBe(
      "https://integrate.api.nvidia.com/v1",
    );
    expect(byName.zhipu_glm.settingsConfig.model).toBe("glm-5.1");
  });

  it("uses the OpenAI Chat format by default", () => {
    expect(ACCIO_DEFAULT_API_FORMAT).toBe("openai_chat");
  });

  it("ships reasoning models with selectable effort levels", () => {
    expect(ACCIO_DEEPSEEK_MODELS.map((model) => model.id)).toEqual([
      "deepseek-chat",
      "deepseek-v4-flash",
      "deepseek-v4-pro",
    ]);
    expect(ACCIO_DEEPSEEK_MODELS[1].reasoning_efforts).toEqual([
      "low",
      "medium",
      "high",
    ]);
    expect(ACCIO_DEEPSEEK_MODELS[1].context_window).toBe(1_000_000);
    expect(ACCIO_DEEPSEEK_MODELS[1].default_reasoning_effort).toBe("medium");
    expect(ACCIO_DEEPSEEK_MODELS[2].context_window).toBe(1_000_000);
    expect(ACCIO_DEEPSEEK_MODELS[2].default_reasoning_effort).toBe("high");
  });
});
