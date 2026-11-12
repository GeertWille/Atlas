import { z } from "zod";
import type { ScanOptions } from "@/bindings";
import { OBSERVE_SECONDS } from "@/shared/domain/scan-options";
import { normalizeScanUrl } from "./scan-url";

const scanFormSchema = z.object({
  url: z.string().transform((value, context) => {
    const result = normalizeScanUrl(value);
    if (result.ok) return result.url;
    context.addIssue({ code: "custom", message: result.error });
    return z.NEVER;
  }),
  device: z.enum(["desktop", "mobile"]),
  observeSeconds: z.number().int().min(OBSERVE_SECONDS.min).max(OBSERVE_SECONDS.max),
  autoScroll: z.boolean(),
  acceptConsent: z.boolean(),
  aiEnabled: z.boolean(),
});

export type ScanForm = z.input<typeof scanFormSchema>;

export type ScanFormResult = { ok: true; options: ScanOptions } | { ok: false; urlError: string };

export function validateScanForm(form: ScanForm): ScanFormResult {
  const parsed = scanFormSchema.safeParse(form);
  if (parsed.success) return { ok: true, options: parsed.data };
  return { ok: false, urlError: parsed.error.issues[0]?.message ?? "Check the scan options." };
}
