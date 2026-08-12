import { useQuery } from "@tanstack/react-query";
import { providersApi } from "@/lib/api/providers";

export const accioKeys = {
  all: ["accio"] as const,
  liveProviderIds: ["accio", "liveProviderIds"] as const,
};

export function useAccioLiveProviderIds(enabled: boolean) {
  return useQuery({
    queryKey: accioKeys.liveProviderIds,
    queryFn: () => providersApi.getAccioLiveProviderIds(),
    enabled,
  });
}
