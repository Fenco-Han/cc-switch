import { useCallback, useMemo, useState } from "react";
import type { AppId } from "@/lib/api";
import { useProvidersQuery } from "@/lib/query/queries";
import {
  ACCIO_DEFAULT_API_FORMAT,
  ACCIO_DEEPSEEK_MODELS,
  ACCIO_REASONING_EFFORTS,
  type AccioApiFormat,
  type AccioModelConfig,
  type AccioReasoningEffort,
  type AccioProviderSettingsConfig,
} from "@/config/accioProviderPresets";

interface UseAccioFormStateParams {
  initialData?: {
    settingsConfig?: Record<string, unknown>;
  };
  appId: AppId;
  providerId?: string;
  onSettingsConfigChange: (config: string) => void;
  getSettingsConfig: () => string;
}

const ACCIO_DEFAULT_CONFIG_OBJ = {
  name: "",
  base_url: "",
  api_key: "",
  api_format: ACCIO_DEFAULT_API_FORMAT,
  model: "",
  models: [],
} satisfies AccioProviderSettingsConfig;

export const ACCIO_DEFAULT_CONFIG = JSON.stringify(
  ACCIO_DEFAULT_CONFIG_OBJ,
  null,
  2,
);

const ACCIO_DEEPSEEK_V4_LEGACY_CONTEXT_WINDOW = 131_072;
const ACCIO_DEEPSEEK_V4_CONTEXT_WINDOW = 1_000_000;

function normalizeAccioContextWindow(
  id: string,
  contextWindow: number | undefined,
): number | undefined {
  if (
    contextWindow === ACCIO_DEEPSEEK_V4_LEGACY_CONTEXT_WINDOW &&
    /^deepseek-v4-(flash|pro)$/i.test(id)
  ) {
    return ACCIO_DEEPSEEK_V4_CONTEXT_WINDOW;
  }
  return contextWindow;
}

export interface AccioFormState {
  accioProviderKey: string;
  setAccioProviderKey: (key: string) => void;
  accioBaseUrl: string;
  accioApiKey: string;
  accioApiFormat: AccioApiFormat;
  accioModel: string;
  accioModels: AccioModelConfig[];
  existingAccioKeys: string[];
  handleAccioBaseUrlChange: (baseUrl: string) => void;
  handleAccioApiKeyChange: (apiKey: string) => void;
  handleAccioApiFormatChange: (apiFormat: AccioApiFormat) => void;
  handleAccioModelChange: (model: string) => void;
  handleAccioModelsChange: (models: AccioModelConfig[]) => void;
  resetAccioState: (config?: Partial<AccioProviderSettingsConfig>) => void;
}

function parseAccioField<T>(
  initialData: UseAccioFormStateParams["initialData"],
  field: string,
  fallback: T,
): T {
  if (initialData?.settingsConfig) {
    return (initialData.settingsConfig[field] as T | undefined) ?? fallback;
  }
  return (
    ((ACCIO_DEFAULT_CONFIG_OBJ as Record<string, unknown>)[field] as
      | T
      | undefined) ?? fallback
  );
}

export function normalizeAccioModels(raw: unknown): AccioModelConfig[] {
  const rawModels =
    Array.isArray(raw) || !raw || typeof raw !== "object"
      ? raw
      : (raw as Record<string, unknown>).models;

  if (Array.isArray(rawModels)) {
    const models = rawModels.flatMap((entry): AccioModelConfig[] => {
      if (!entry || typeof entry !== "object") return [];
      const value = entry as Record<string, unknown>;
      const id = typeof value.id === "string" ? value.id.trim() : "";
      const name =
        typeof value.name === "string" && value.name.trim()
          ? value.name.trim()
          : id;
      if (!id) return [];

      const reasoningEfforts = Array.isArray(value.reasoning_efforts)
        ? value.reasoning_efforts.filter(
            (effort): effort is AccioReasoningEffort =>
              ACCIO_REASONING_EFFORTS.includes(effort as AccioReasoningEffort),
          )
        : [];
      const defaultEffort =
        typeof value.default_reasoning_effort === "string" &&
        ACCIO_REASONING_EFFORTS.includes(
          value.default_reasoning_effort as AccioReasoningEffort,
        )
          ? (value.default_reasoning_effort as AccioReasoningEffort)
          : reasoningEfforts[0];
      const contextWindow = normalizeAccioContextWindow(
        id,
        typeof value.context_window === "number" &&
          Number.isFinite(value.context_window) &&
          value.context_window > 0
          ? value.context_window
          : undefined,
      );

      return [
        {
          id,
          name,
          ...(reasoningEfforts.length > 0
            ? { reasoning_efforts: reasoningEfforts }
            : {}),
          ...(defaultEffort ? { default_reasoning_effort: defaultEffort } : {}),
          ...(contextWindow ? { context_window: contextWindow } : {}),
        },
      ];
    });
    if (models.length > 0) return models;
  }

  const config =
    raw && typeof raw === "object"
      ? (raw as Record<string, unknown>)
      : undefined;
  const model = typeof config?.model === "string" ? config.model.trim() : "";
  const name = typeof config?.name === "string" ? config.name.trim() : "";
  if (name === "deepseek" || model === "deepseek-chat") {
    return ACCIO_DEEPSEEK_MODELS;
  }
  return model ? [{ id: model, name: model }] : [];
}

export function useAccioFormState({
  initialData,
  appId,
  providerId,
  onSettingsConfigChange,
  getSettingsConfig,
}: UseAccioFormStateParams): AccioFormState {
  const { data: accioProvidersData } = useProvidersQuery("accio");
  const existingAccioKeys = useMemo(() => {
    if (!accioProvidersData?.providers) return [];
    return Object.keys(accioProvidersData.providers).filter(
      (key) => key !== providerId,
    );
  }, [accioProvidersData?.providers, providerId]);

  const [accioProviderKey, setAccioProviderKey] = useState<string>(() => {
    if (appId !== "accio") return "";
    return providerId || "";
  });
  const [accioBaseUrl, setAccioBaseUrl] = useState<string>(() => {
    if (appId !== "accio") return "";
    return parseAccioField(initialData, "base_url", "");
  });
  const [accioApiKey, setAccioApiKey] = useState<string>(() => {
    if (appId !== "accio") return "";
    return parseAccioField(initialData, "api_key", "");
  });
  const [accioApiFormat, setAccioApiFormat] = useState<AccioApiFormat>(() => {
    if (appId !== "accio") return ACCIO_DEFAULT_API_FORMAT;
    return parseAccioField<AccioApiFormat>(
      initialData,
      "api_format",
      ACCIO_DEFAULT_API_FORMAT,
    );
  });
  const [accioModel, setAccioModel] = useState<string>(() => {
    if (appId !== "accio") return "";
    return parseAccioField(initialData, "model", "");
  });
  const [accioModels, setAccioModels] = useState<AccioModelConfig[]>(() => {
    if (appId !== "accio") return [];
    return normalizeAccioModels(initialData?.settingsConfig);
  });

  const updateAccioConfig = useCallback(
    (updater: (config: Record<string, unknown>) => void) => {
      try {
        const config = JSON.parse(
          getSettingsConfig() || ACCIO_DEFAULT_CONFIG,
        ) as Record<string, unknown>;
        updater(config);
        onSettingsConfigChange(JSON.stringify(config, null, 2));
      } catch {
        // The JSON editor owns hard validation and will surface malformed input.
      }
    },
    [getSettingsConfig, onSettingsConfigChange],
  );

  const handleAccioBaseUrlChange = useCallback(
    (baseUrl: string) => {
      setAccioBaseUrl(baseUrl);
      updateAccioConfig((config) => {
        config.base_url = baseUrl.trim().replace(/\/+$/, "");
      });
    },
    [updateAccioConfig],
  );

  const handleAccioApiKeyChange = useCallback(
    (apiKey: string) => {
      setAccioApiKey(apiKey);
      updateAccioConfig((config) => {
        config.api_key = apiKey;
      });
    },
    [updateAccioConfig],
  );

  const handleAccioApiFormatChange = useCallback(
    (apiFormat: AccioApiFormat) => {
      setAccioApiFormat(apiFormat);
      updateAccioConfig((config) => {
        config.api_format = apiFormat;
      });
    },
    [updateAccioConfig],
  );

  const handleAccioModelChange = useCallback(
    (model: string) => {
      setAccioModel(model);
      updateAccioConfig((config) => {
        if (model.trim()) {
          config.model = model.trim();
        } else {
          delete config.model;
        }
      });
    },
    [updateAccioConfig],
  );

  const handleAccioModelsChange = useCallback(
    (models: AccioModelConfig[]) => {
      setAccioModels(models);
      const nextDefaultModel = models.some((item) => item.id === accioModel)
        ? accioModel
        : (models[0]?.id ?? "");
      setAccioModel(nextDefaultModel);
      updateAccioConfig((config) => {
        if (models.length === 0) {
          delete config.models;
          delete config.model;
        } else {
          config.models = models;
          config.model = nextDefaultModel;
        }
      });
    },
    [accioModel, updateAccioConfig],
  );

  const resetAccioState = useCallback(
    (config?: Partial<AccioProviderSettingsConfig>) => {
      const models = normalizeAccioModels(config);
      setAccioProviderKey("");
      setAccioBaseUrl(config?.base_url || "");
      setAccioApiKey(config?.api_key || "");
      setAccioApiFormat(config?.api_format ?? ACCIO_DEFAULT_API_FORMAT);
      setAccioModels(models);
      setAccioModel(config?.model || models[0]?.id || "");
    },
    [],
  );

  return {
    accioProviderKey,
    setAccioProviderKey,
    accioBaseUrl,
    accioApiKey,
    accioApiFormat,
    accioModel,
    accioModels,
    existingAccioKeys,
    handleAccioBaseUrlChange,
    handleAccioApiKeyChange,
    handleAccioApiFormatChange,
    handleAccioModelChange,
    handleAccioModelsChange,
    resetAccioState,
  };
}
