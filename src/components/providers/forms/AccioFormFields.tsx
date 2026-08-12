import { useRef } from "react";
import { useTranslation } from "react-i18next";
import { Plus, Trash2 } from "lucide-react";
import { FormLabel } from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  ACCIO_REASONING_EFFORTS,
  type AccioApiFormat,
  type AccioModelConfig,
  type AccioReasoningEffort,
} from "@/config/accioProviderPresets";
import type { ProviderCategory } from "@/types";
import { ApiKeySection } from "./shared";

interface AccioFormFieldsProps {
  baseUrl: string;
  onBaseUrlChange: (value: string) => void;
  apiKey: string;
  onApiKeyChange: (value: string) => void;
  apiFormat: AccioApiFormat;
  onApiFormatChange: (value: AccioApiFormat) => void;
  model: string;
  onModelChange: (value: string) => void;
  models: AccioModelConfig[];
  onModelsChange: (models: AccioModelConfig[]) => void;
  category?: ProviderCategory;
  shouldShowApiKeyLink: boolean;
  websiteUrl: string;
}

function sortReasoningEfforts(
  efforts: AccioReasoningEffort[],
): AccioReasoningEffort[] {
  return ACCIO_REASONING_EFFORTS.filter((effort) => efforts.includes(effort));
}

export function AccioFormFields({
  baseUrl,
  onBaseUrlChange,
  apiKey,
  onApiKeyChange,
  apiFormat,
  onApiFormatChange,
  model,
  onModelChange,
  models,
  onModelsChange,
  category,
  shouldShowApiKeyLink,
  websiteUrl,
}: AccioFormFieldsProps) {
  const { t } = useTranslation();
  const modelKeysRef = useRef<string[]>([]);

  while (modelKeysRef.current.length < models.length) {
    modelKeysRef.current.push(crypto.randomUUID());
  }
  if (modelKeysRef.current.length > models.length) {
    modelKeysRef.current.length = models.length;
  }

  const addModel = () => {
    modelKeysRef.current.push(crypto.randomUUID());
    onModelsChange([
      ...models,
      {
        id: "",
        name: "",
        reasoning_efforts: [],
        context_window: 65_536,
      },
    ]);
  };

  const removeModel = (index: number) => {
    modelKeysRef.current.splice(index, 1);
    const next = [...models];
    next.splice(index, 1);
    onModelsChange(next);
  };

  const updateModel = (index: number, patch: Partial<AccioModelConfig>) => {
    onModelsChange(
      models.map((item, itemIndex) =>
        itemIndex === index ? { ...item, ...patch } : item,
      ),
    );
  };

  const toggleReasoningEffort = (
    index: number,
    effort: AccioReasoningEffort,
    checked: boolean,
  ) => {
    const current = models[index]?.reasoning_efforts ?? [];
    const nextEfforts = sortReasoningEfforts(
      checked
        ? [...current, effort]
        : current.filter((item) => item !== effort),
    );
    const currentDefault = models[index]?.default_reasoning_effort;
    const nextDefault =
      currentDefault && nextEfforts.includes(currentDefault)
        ? currentDefault
        : nextEfforts[0];
    updateModel(index, {
      reasoning_efforts: nextEfforts,
      default_reasoning_effort: nextDefault,
    });
  };

  return (
    <>
      <div className="space-y-2">
        <FormLabel htmlFor="accio-base-url">
          {t("accio.form.baseUrl", { defaultValue: "API Endpoint" })}
        </FormLabel>
        <Input
          id="accio-base-url"
          value={baseUrl}
          onChange={(event) => onBaseUrlChange(event.target.value)}
          placeholder="https://api.deepseek.com"
          autoComplete="off"
        />
      </div>

      <ApiKeySection
        id="accio-api-key"
        value={apiKey}
        onChange={onApiKeyChange}
        category={category}
        shouldShowLink={shouldShowApiKeyLink}
        websiteUrl={websiteUrl}
      />

      <div className="space-y-2">
        <FormLabel htmlFor="accio-api-format">
          {t("accio.form.apiFormat", { defaultValue: "API Protocol" })}
        </FormLabel>
        <Select value={apiFormat} onValueChange={onApiFormatChange}>
          <SelectTrigger id="accio-api-format">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="openai_chat">OpenAI Chat</SelectItem>
            <SelectItem value="adk">Aliyun ADK</SelectItem>
          </SelectContent>
        </Select>
      </div>

      <div className="space-y-3 border-l border-border-default pl-3">
        <div className="flex items-center justify-between gap-3">
          <FormLabel>
            {t("accio.form.models", { defaultValue: "模型配置" })}
          </FormLabel>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={addModel}
            className="h-7 gap-1"
          >
            <Plus className="h-3.5 w-3.5" />
            {t("accio.form.addModel", { defaultValue: "添加模型" })}
          </Button>
        </div>

        <div className="space-y-2">
          <FormLabel htmlFor="accio-default-model">
            {t("accio.form.defaultModel", { defaultValue: "默认模型" })}
          </FormLabel>
          <Select value={model} onValueChange={onModelChange}>
            <SelectTrigger id="accio-default-model">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {models
                .filter((item) => item.id)
                .map((item, index) => {
                  const key = modelKeysRef.current[index];
                  return (
                    <SelectItem
                      key={`${index}-${item.id || key}`}
                      value={item.id}
                    >
                      {item.name || item.id}
                    </SelectItem>
                  );
                })}
            </SelectContent>
          </Select>
        </div>

        {models.length === 0 ? (
          <p className="py-2 text-sm text-muted-foreground">
            {t("accio.form.noModels", { defaultValue: "暂无模型配置" })}
          </p>
        ) : (
          <div className="space-y-2">
            {models.map((item, index) => {
              const key = modelKeysRef.current[index];
              const efforts = item.reasoning_efforts ?? [];
              return (
                <div
                  key={key}
                  className="space-y-3 rounded-md border border-border p-3"
                >
                  <div className="grid grid-cols-[1fr_1fr_auto] items-start gap-2">
                    <div className="space-y-1 min-w-0">
                      <FormLabel htmlFor={`accio-model-id-${key}`}>
                        {t("accio.form.modelId", { defaultValue: "模型 ID" })}
                      </FormLabel>
                      <Input
                        id={`accio-model-id-${key}`}
                        value={item.id}
                        onChange={(event) =>
                          updateModel(index, {
                            id: event.target.value.trim(),
                          })
                        }
                        placeholder="deepseek-v4-flash"
                        className="h-8"
                      />
                    </div>
                    <div className="space-y-1 min-w-0">
                      <FormLabel htmlFor={`accio-model-name-${key}`}>
                        {t("accio.form.modelName", {
                          defaultValue: "显示名称",
                        })}
                      </FormLabel>
                      <Input
                        id={`accio-model-name-${key}`}
                        value={item.name}
                        onChange={(event) =>
                          updateModel(index, { name: event.target.value })
                        }
                        placeholder="DeepSeek V4 Flash"
                        className="h-8"
                      />
                    </div>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      onClick={() => removeModel(index)}
                      aria-label={t("accio.form.removeModel", {
                        defaultValue: "移除模型",
                      })}
                      className="mt-5 h-8 w-8 text-muted-foreground"
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  </div>

                  <div className="grid gap-3 lg:grid-cols-[1fr_160px]">
                    <div className="space-y-1.5">
                      <FormLabel>
                        {t("accio.form.reasoningEfforts", {
                          defaultValue: "思考等级",
                        })}
                      </FormLabel>
                      <div className="flex flex-wrap gap-4">
                        {ACCIO_REASONING_EFFORTS.map((effort) => {
                          const checked = efforts.includes(effort);
                          return (
                            <label
                              key={effort}
                              className="flex items-center gap-1.5 text-sm"
                            >
                              <Checkbox
                                checked={checked}
                                onCheckedChange={(value) =>
                                  toggleReasoningEffort(
                                    index,
                                    effort,
                                    value === true,
                                  )
                                }
                              />
                              <span className="capitalize">{effort}</span>
                            </label>
                          );
                        })}
                      </div>
                    </div>

                    <div className="space-y-1.5">
                      <FormLabel htmlFor={`accio-default-effort-${key}`}>
                        {t("accio.form.defaultReasoningEffort", {
                          defaultValue: "默认等级",
                        })}
                      </FormLabel>
                      <Select
                        value={item.default_reasoning_effort ?? ""}
                        onValueChange={(value) =>
                          updateModel(index, {
                            default_reasoning_effort:
                              value as AccioReasoningEffort,
                          })
                        }
                        disabled={efforts.length === 0}
                      >
                        <SelectTrigger id={`accio-default-effort-${key}`}>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {efforts.map((effort) => (
                            <SelectItem key={effort} value={effort}>
                              <span className="capitalize">{effort}</span>
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </div>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </>
  );
}
