import { describe, expect, it } from "vitest";

import {
  accioProviderPresets,
  ACCIO_DEFAULT_API_FORMAT,
  ACCIO_DEEPSEEK_MODELS,
} from "./accioProviderPresets";

describe("Accio provider presets", () => {
  it("ships one minimal DeepSeek preset", () => {
    expect(accioProviderPresets).toHaveLength(1);
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
