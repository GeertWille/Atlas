import type { Settings } from "@/bindings";

export type AiAvailability = {
  /** Who classifies the scripts the vendor rules cannot place. `null`: nobody, the rules stand alone. */
  triage: "jev" | "claude" | null;
  /** Phase 2 logic analysis needs an Anthropic key. */
  logicAnalysis: boolean;
};

type KeyState = Pick<Settings, "hasApiKey" | "hasTypesafeKey" | "jevEnabled">;

/** Which AI steps a scan started now would run, given the stored keys and the Jev switch. */
export function aiAvailability(settings: KeyState): AiAvailability {
  const jev = settings.jevEnabled && settings.hasTypesafeKey;
  return {
    triage: jev ? "jev" : settings.hasApiKey ? "claude" : null,
    logicAnalysis: settings.hasApiKey,
  };
}

/** Logic analysis never runs without triage, so triage alone decides whether a scan can use AI at all. */
export function anyAiAvailable(settings: KeyState): boolean {
  return aiAvailability(settings).triage !== null;
}
