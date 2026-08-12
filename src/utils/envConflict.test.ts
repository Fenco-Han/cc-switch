import { describe, expect, it, beforeEach } from "vitest";
import type { EnvConflict } from "@/types/env";
import {
  dismissEnvConflicts,
  envConflictId,
  forgetDismissedEnvConflicts,
  hasUndismissedEnvConflicts,
  maskEnvConflictValue,
} from "./envConflict";

const conflict = (varName: string, varValue = "secret-value"): EnvConflict => ({
  varName,
  varValue,
  sourceType: "system",
  sourcePath: "Process Environment",
});

describe("envConflict", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("masks credential-like environment variables", () => {
    expect(maskEnvConflictValue(conflict("XAI_API_KEY"))).toBe("••••••••");
    expect(maskEnvConflictValue(conflict("ANTHROPIC_AUTH_TOKEN"))).toBe(
      "••••••••",
    );
    expect(maskEnvConflictValue(conflict("GROK_DEFAULT_MODEL", "grok-4"))).toBe(
      "grok-4",
    );
  });

  it("remembers dismissed conflicts across app reloads", () => {
    const target = conflict("XAI_API_KEY");

    dismissEnvConflicts([target]);

    expect(hasUndismissedEnvConflicts([target])).toBe(false);
    expect(
      hasUndismissedEnvConflicts([
        conflict("XAI_API_KEY"),
        conflict("GROK_DEFAULT_MODEL", "grok-4"),
      ]),
    ).toBe(true);
  });

  it("forgets only conflicts that were removed", () => {
    const oldConflict = conflict("XAI_API_KEY");
    const newConflict = conflict("GROK_DEFAULT_MODEL", "grok-4");

    dismissEnvConflicts([oldConflict, newConflict]);
    forgetDismissedEnvConflicts([oldConflict]);

    expect(hasUndismissedEnvConflicts([newConflict])).toBe(false);
    expect(hasUndismissedEnvConflicts([oldConflict])).toBe(true);
  });

  it("uses variable and source path as the dismissal identity", () => {
    const first = conflict("XAI_API_KEY");
    const second = {
      ...conflict("XAI_API_KEY"),
      sourcePath: "/Users/test/.zshrc:42",
      sourceType: "file" as const,
    };

    expect(envConflictId(first)).not.toBe(envConflictId(second));
  });
});
