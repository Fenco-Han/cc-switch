import { describe, expect, it } from "vitest";

import { normalizeAccioModels } from "./useAccioFormState";

describe("normalizeAccioModels", () => {
  it("expands legacy DeepSeek configs into the three supported models", () => {
    expect(
      normalizeAccioModels({
        name: "deepseek",
        model: "deepseek-chat",
      }).map((model) => model.id),
    ).toEqual(["deepseek-chat", "deepseek-v4-flash", "deepseek-v4-pro"]);
  });

  it("keeps explicit model metadata and filters unsupported effort values", () => {
    const models = normalizeAccioModels({
      model: "deepseek-v4-flash",
      models: [
        {
          id: "deepseek-v4-flash",
          name: "Flash",
          reasoning_efforts: ["low", "extreme", "high"],
          default_reasoning_effort: "extreme",
          context_window: 128_000,
        },
      ],
    });

    expect(models).toEqual([
      {
        id: "deepseek-v4-flash",
        name: "Flash",
        reasoning_efforts: ["low", "high"],
        default_reasoning_effort: "low",
        context_window: 128_000,
      },
    ]);
  });

  it("upgrades the legacy DeepSeek V4 context window to 1M", () => {
    expect(
      normalizeAccioModels({
        models: [
          {
            id: "deepseek-v4-flash",
            context_window: 131_072,
          },
          {
            id: "deepseek-v4-pro",
            context_window: 131_072,
          },
        ],
      }),
    ).toEqual([
      {
        id: "deepseek-v4-flash",
        name: "deepseek-v4-flash",
        context_window: 1_000_000,
      },
      {
        id: "deepseek-v4-pro",
        name: "deepseek-v4-pro",
        context_window: 1_000_000,
      },
    ]);
  });

  it("returns a legacy single-model entry for unknown providers", () => {
    expect(
      normalizeAccioModels({
        name: "other",
        model: "other-model",
      }),
    ).toEqual([{ id: "other-model", name: "other-model" }]);
  });
});
