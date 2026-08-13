import { useCallback, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronRight, Download, Loader2, Plus, Trash2 } from "lucide-react";
import { toast } from "sonner";
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
  type AccioModelConfig,
  type AccioReasoningEffort,
} from "@/config/accioProviderPresets";
import type { ProviderCategory } from "@/types";
import {
  fetchModelsForConfig,
  showFetchModelsError,
  type FetchedModel,
} from "@/lib/api/model-fetch";
import { ApiKeySection, ModelDropdown } from "./shared";

interface AccioFormFieldsProps {
  baseUrl: string;
  onBaseUrlChange: (value: string) => void;
  apiKey: string;
  onApiKeyChange: (value: string) => void;
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
  const [expandedModelKeys, setExpandedModelKeys] = useState<Set<string>>(
    new Set(),
  );
  const [fetchedModels, setFetchedModels] = useState<FetchedModel[]>([]);
  const [isFetchingModels, setIsFetchingModels] = useState(false);

  while (modelKeysRef.current.length < models.length) {
    modelKeysRef.current.push(crypto.randomUUID());
  }
  if (modelKeysRef.current.length > models.length) {
    modelKeysRef.current.length = models.length;
  }
  const modelKeys = modelKeysRef.current;

  const addModel = () => {
    modelKeysRef.current.push(crypto.randomUUID());
    onModelsChange([...models, { id: "", name: "" }]);
  };

  const removeModel = (index: number) => {
    const removedKey = modelKeysRef.current[index];
    modelKeysRef.current.splice(index, 1);
    const next = [...models];
    next.splice(index, 1);
    onModelsChange(next);
    setExpandedModelKeys((current) => {
      const updated = new Set(current);
      updated.delete(removedKey);
      return updated;
    });
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

  const toggleModelDetails = (modelKey: string) => {
    setExpandedModelKeys((current) => {
      const next = new Set(current);
      if (next.has(modelKey)) next.delete(modelKey);
      else next.add(modelKey);
      return next;
    });
  };

  const handleFetchModels = useCallback(() => {
    if (!baseUrl || !apiKey) {
      showFetchModelsError(null, t, {
        hasApiKey: !!apiKey,
        hasBaseUrl: !!baseUrl,
      });
      return;
    }
    setIsFetchingModels(true);
    fetchModelsForConfig(baseUrl, apiKey)
      .then((fetched) => {
        setFetchedModels(fetched);
        if (fetched.length === 0) {
          toast.info(t("providerForm.fetchModelsEmpty"));
        } else {
          toast.success(
            t("providerForm.fetchModelsSuccess", { count: fetched.length }),
          );
        }
      })
      .catch((err) => {
        console.warn("[ModelFetch] Failed:", err);
        showFetchModelsError(err, t);
      })
      .finally(() => setIsFetchingModels(false));
  }, [baseUrl, apiKey, t]);

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
          placeholder="https://api.example.com/v1"
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

      <div className="space-y-3 border-l border-border-default pl-3">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-normal leading-5">
            {t("accio.form.models", { defaultValue: "模型配置" })}
          </h3>
          <div className="flex gap-1">
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={handleFetchModels}
              disabled={isFetchingModels}
              className="h-7 gap-1"
            >
              {isFetchingModels ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Download className="h-3.5 w-3.5" />
              )}
              {t("providerForm.fetchModels")}
            </Button>
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
            <div className="flex items-center gap-2 px-1 text-xs text-muted-foreground">
              <span className="w-9" />
              <span className="flex-1">
                {t("accio.form.modelId", { defaultValue: "模型 ID" })}
              </span>
              <span className="flex-1">
                {t("accio.form.modelName", { defaultValue: "显示名称" })}
              </span>
              <span className="w-9" />
            </div>
            {models.map((item, index) => {
              const modelKey = modelKeys[index];
              const isExpanded = expandedModelKeys.has(modelKey);
              const detailsId = `accio-model-details-${modelKey}`;
              const efforts = item.reasoning_efforts ?? [];
              return (
                <div key={modelKey} className="space-y-2">
                  <div className="flex items-center gap-2">
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      onClick={() => toggleModelDetails(modelKey)}
                      aria-label={t("accio.form.toggleModelDetails", {
                        defaultValue: "展开或收起模型详情",
                      })}
                      aria-expanded={isExpanded}
                      aria-controls={detailsId}
                      className="h-9 w-9 shrink-0"
                    >
                      <ChevronRight
                        className={`h-4 w-4 transition-transform motion-reduce:transition-none ${
                          isExpanded ? "rotate-90" : ""
                        }`}
                      />
                    </Button>
                    <div className="flex min-w-0 flex-1 items-center gap-1">
                      <Input
                        id={`accio-model-id-${modelKey}`}
                        value={item.id}
                        onChange={(e) =>
                          updateModel(index, { id: e.target.value.trim() })
                        }
                        placeholder={t("accio.form.modelIdPlaceholder", {
                          defaultValue: "gpt-4o-mini",
                        })}
                        aria-label={t("accio.form.modelId", {
                          defaultValue: "模型 ID",
                        })}
                        className="min-w-0 flex-1"
                      />
                      {fetchedModels.length > 0 && (
                        <ModelDropdown
                          models={fetchedModels}
                          onSelect={(id) => updateModel(index, { id })}
                        />
                      )}
                    </div>
                    <Input
                      id={`accio-model-name-${modelKey}`}
                      value={item.name}
                      onChange={(e) =>
                        updateModel(index, { name: e.target.value })
                      }
                      placeholder={t("accio.form.modelNamePlaceholder", {
                        defaultValue: "GPT-4o Mini",
                      })}
                      aria-label={t("accio.form.modelName", {
                        defaultValue: "显示名称",
                      })}
                      className="min-w-0 flex-1"
                    />
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      onClick={() => removeModel(index)}
                      aria-label={t("accio.form.removeModel", {
                        defaultValue: "移除模型",
                      })}
                      className="h-9 w-9 shrink-0 text-muted-foreground hover:text-destructive"
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  </div>

                  {isExpanded && (
                    <div
                      id={detailsId}
                      className="ml-9 space-y-3 border-l border-border-default pl-4"
                    >
                      <div className="space-y-1.5">
                        <FormLabel htmlFor={`accio-model-context-${modelKey}`}>
                          {t("accio.form.contextWindow", {
                            defaultValue: "上下文窗口",
                          })}
                        </FormLabel>
                        <Input
                          id={`accio-model-context-${modelKey}`}
                          type="text"
                          inputMode="numeric"
                          pattern="[0-9]*"
                          value={item.context_window ?? ""}
                          onChange={(event) => {
                            const value = event.target.value.replace(
                              /[^\d]/g,
                              "",
                            );
                            updateModel(index, {
                              context_window: value
                                ? parseInt(value, 10)
                                : undefined,
                            });
                          }}
                          placeholder="65536"
                        />
                      </div>

                      <div className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_160px]">
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
                          <FormLabel
                            htmlFor={`accio-default-effort-${modelKey}`}
                          >
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
                            <SelectTrigger id={`accio-default-effort-${modelKey}`}>
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
                  )}
                </div>
              );
            })}
          </div>
        )}

        <p className="text-xs text-muted-foreground">
          {t("accio.form.modelsHint", {
            defaultValue:
              "填写 API Key 后可点击「获取模型列表」自动拉取模型；第一个模型会设为 Accio 默认模型。",
          })}
        </p>
      </div>
    </>
  );
}
