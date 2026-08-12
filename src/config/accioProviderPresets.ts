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
];
