import type { EnvConflict } from "@/types/env";

const DISMISSED_STORAGE_KEY = "cc-switch-env-conflicts-dismissed-v1";

const SENSITIVE_ENV_NAME =
  /(?:API_KEY|AUTH_TOKEN|ACCESS_TOKEN|PASSWORD|SECRET|CREDENTIAL|PRIVATE_KEY)(?:_|$)/i;

export function envConflictId(conflict: EnvConflict): string {
  return `${conflict.varName}\u0000${conflict.sourcePath}`;
}

export function maskEnvConflictValue(conflict: EnvConflict): string {
  return SENSITIVE_ENV_NAME.test(conflict.varName)
    ? "\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022"
    : conflict.varValue;
}

function readDismissedIds(): Set<string> {
  try {
    const raw = localStorage.getItem(DISMISSED_STORAGE_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : [];
    if (!Array.isArray(parsed)) {
      return new Set();
    }
    return new Set(
      parsed.filter((value): value is string => typeof value === "string"),
    );
  } catch {
    return new Set();
  }
}

function writeDismissedIds(ids: Set<string>): void {
  try {
    localStorage.setItem(
      DISMISSED_STORAGE_KEY,
      JSON.stringify(Array.from(ids).sort()),
    );
  } catch {
    // Storage can be unavailable in hardened or private browsing contexts.
  }
}

export function hasUndismissedEnvConflicts(conflicts: EnvConflict[]): boolean {
  const dismissed = readDismissedIds();
  return conflicts.some((conflict) => !dismissed.has(envConflictId(conflict)));
}

export function dismissEnvConflicts(conflicts: EnvConflict[]): void {
  const dismissed = readDismissedIds();
  for (const conflict of conflicts) {
    dismissed.add(envConflictId(conflict));
  }
  writeDismissedIds(dismissed);
}

export function forgetDismissedEnvConflicts(conflicts: EnvConflict[]): void {
  const dismissed = readDismissedIds();
  for (const conflict of conflicts) {
    dismissed.delete(envConflictId(conflict));
  }
  writeDismissedIds(dismissed);
}
