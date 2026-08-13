import type { ProviderCategory } from "../types";
import type { PresetTheme, TemplateValueConfig } from "./claudeProviderPresets";

export type AccioApiFormat = "adk" | "openai_chat";
export type AccioReasoningEffort = "low" | "medium" | "high";

export interface AccioModelConfig {
  id: string;
  name: string;
  reasoning_efforts?: AccioReasoningEffort[];
  default_reasoning_effort?: AccioReasoningEffort;
  context_window?: number;
}

export interface AccioProviderSettingsConfig {
  name: string;
  base_url?: string;
  api_key?: string;
  api_format?: AccioApiFormat;
  model?: string;
  models?: AccioModelConfig[];
  [key: string]: unknown;
}

export interface AccioProviderPreset {
  name: string;
  nameKey?: string;
  websiteUrl: string;
  apiKeyUrl?: string;
  settingsConfig: AccioProviderSettingsConfig;
  category?: ProviderCategory;
  isOfficial?: boolean;
  isPartner?: boolean;
  primePartner?: boolean;
  partnerPromotionKey?: string;
  templateValues?: Record<string, TemplateValueConfig>;
  theme?: PresetTheme;
  icon?: string;
  iconColor?: string;
}

export const ACCIO_DEFAULT_API_FORMAT: AccioApiFormat = "openai_chat";

export const ACCIO_REASONING_EFFORTS: AccioReasoningEffort[] = [
  "low",
  "medium",
  "high",
];

export const ACCIO_DEEPSEEK_MODELS: AccioModelConfig[] = [
  {
    id: "deepseek-chat",
    name: "DeepSeek Chat",
    reasoning_efforts: [],
    context_window: 65_536,
  },
  {
    id: "deepseek-v4-flash",
    name: "DeepSeek V4 Flash",
    reasoning_efforts: ["low", "medium", "high"],
    default_reasoning_effort: "medium",
    context_window: 1_000_000,
  },
  {
    id: "deepseek-v4-pro",
    name: "DeepSeek V4 Pro",
    reasoning_efforts: ["low", "medium", "high"],
    default_reasoning_effort: "high",
    context_window: 1_000_000,
  },
];

export const accioProviderPresets: AccioProviderPreset[] = [
  {
    name: "DeepSeek",
    websiteUrl: "https://platform.deepseek.com",
    apiKeyUrl: "https://platform.deepseek.com/api_keys",
    settingsConfig: {
      name: "deepseek",
      base_url: "https://api.deepseek.com",
      api_key: "",
      api_format: "openai_chat",
      model: "deepseek-chat",
      models: ACCIO_DEEPSEEK_MODELS,
    },
    category: "cn_official",
    icon: "deepseek",
    iconColor: "#4D6BFE",
  },
  {
    name: "Xiaomi MiMo",
    websiteUrl: "https://platform.xiaomimimo.com",
    apiKeyUrl: "https://platform.xiaomimimo.com/#/console/api-keys",
    settingsConfig: {
      name: "xiaomi_mimo",
      base_url: "https://api.xiaomimimo.com/v1",
      api_key: "",
      api_format: "openai_chat",
      model: "mimo-v2.5-pro",
      models: [
        { id: "mimo-v2.5-pro", name: "MiMo v2.5 Pro", context_window: 1_048_576 },
        { id: "mimo-v2.5", name: "MiMo v2.5", context_window: 1_048_576 },
      ],
    },
    category: "cn_official",
    icon: "xiaomimimo",
    iconColor: "#000000",
  },
  {
    name: "Kimi",
    websiteUrl: "https://platform.kimi.com?aff=cc-switch",
    apiKeyUrl: "https://platform.kimi.com/console/api-keys?aff=cc-switch",
    settingsConfig: {
      name: "kimi",
      base_url: "https://api.moonshot.cn/v1",
      api_key: "",
      api_format: "openai_chat",
      model: "kimi-k2.7-code",
      models: [
        { id: "kimi-k2.7-code", name: "Kimi K2.7 Code" },
        { id: "kimi-k3", name: "Kimi K3", context_window: 1_048_576 },
      ],
    },
    category: "cn_official",
    icon: "kimi",
    iconColor: "#6366F1",
  },
  {
    name: "MiniMax",
    websiteUrl: "https://platform.minimaxi.com",
    apiKeyUrl: "https://platform.minimaxi.com/subscribe/coding-plan",
    settingsConfig: {
      name: "minimax",
      base_url: "https://api.minimaxi.com/v1",
      api_key: "",
      api_format: "openai_chat",
      model: "MiniMax-M2.7",
      models: [{ id: "MiniMax-M2.7", name: "MiniMax M2.7" }],
    },
    category: "cn_official",
    icon: "minimax",
    iconColor: "#FF6B6B",
  },
  {
    name: "Zhipu GLM",
    websiteUrl: "https://open.bigmodel.cn",
    apiKeyUrl: "https://www.bigmodel.cn/claude-code?ic=RRVJPB5SII",
    settingsConfig: {
      name: "zhipu_glm",
      base_url: "https://open.bigmodel.cn/api/coding/paas/v4",
      api_key: "",
      api_format: "openai_chat",
      model: "glm-5.1",
      models: [{ id: "glm-5.1", name: "GLM-5.1", context_window: 204_800 }],
    },
    category: "cn_official",
    icon: "zhipu",
    iconColor: "#0F62FE",
  },
  {
    name: "Nvidia",
    websiteUrl: "https://build.nvidia.com",
    apiKeyUrl: "https://build.nvidia.com/settings/api-keys",
    settingsConfig: {
      name: "nvidia",
      base_url: "https://integrate.api.nvidia.com/v1",
      api_key: "",
      api_format: "openai_chat",
      model: "moonshotai/kimi-k2.6",
      models: [{ id: "moonshotai/kimi-k2.6", name: "Moonshot Kimi K2.6" }],
    },
    category: "aggregator",
    icon: "nvidia",
    iconColor: "#000000",
  },
  {
    name: "OpenRouter",
    websiteUrl: "https://openrouter.ai",
    apiKeyUrl: "https://openrouter.ai/keys",
    settingsConfig: {
      name: "openrouter",
      base_url: "https://openrouter.ai/api/v1",
      api_key: "",
      api_format: "openai_chat",
      model: "anthropic/claude-opus-5",
      models: [
        {
          id: "anthropic/claude-opus-5",
          name: "Claude Opus 5",
          context_window: 1_000_000,
        },
        {
          id: "anthropic/claude-sonnet-5",
          name: "Claude Sonnet 5",
          context_window: 1_000_000,
        },
        {
          id: "anthropic/claude-haiku-4-5",
          name: "Claude Haiku 4.5",
          context_window: 200_000,
        },
        {
          id: "openai/gpt-5.6-sol",
          name: "GPT-5.6 Sol",
          context_window: 400_000,
        },
        {
          id: "google/gemini-3.6-flash",
          name: "Gemini 3.6 Flash",
          context_window: 1_000_000,
        },
      ],
    },
    category: "aggregator",
    icon: "openrouter",
    iconColor: "#6366F1",
  },
];
