import { z } from "zod";

export const BUDGET_USD = { min: 0.05, max: 20 } as const;
export const OBSERVE_SECONDS = { min: 5, max: 60 } as const;

const modelId = z
  .string()
  .trim()
  .min(1, "Enter a model id.")
  .regex(/^[a-z0-9][a-z0-9._-]*$/i, "Model ids use letters, digits, dots and dashes only.");

const budgetUsd = z.coerce
  .number("Enter an amount in dollars.")
  .min(BUDGET_USD.min, `The budget must be at least $${BUDGET_USD.min.toFixed(2)}.`)
  .max(BUDGET_USD.max, `The budget cannot exceed $${BUDGET_USD.max.toFixed(2)}.`);

const observeSeconds = z.coerce
  .number("Enter a number of seconds.")
  .int("Use whole seconds.")
  .min(OBSERVE_SECONDS.min, `Observe for at least ${OBSERVE_SECONDS.min} seconds.`)
  .max(OBSERVE_SECONDS.max, `Observe for at most ${OBSERVE_SECONDS.max} seconds.`);

const apiKey = z
  .string()
  .trim()
  .min(1, "Paste your API key.")
  .refine((value) => !/\s/.test(value), "An API key cannot contain spaces.");

export type Parsed<T> = { ok: true; value: T } | { ok: false; error: string };

function parseWith<T>(schema: z.ZodType<T>, input: string): Parsed<T> {
  const result = schema.safeParse(input);
  return result.success
    ? { ok: true, value: result.data }
    : { ok: false, error: result.error.issues[0]?.message ?? "Invalid value." };
}

export const parseModelId = (input: string) => parseWith(modelId, input);
export const parseBudgetUsd = (input: string) => parseWith(budgetUsd, input);
export const parseObserveSeconds = (input: string) => parseWith(observeSeconds, input);
export const parseApiKey = (input: string) => parseWith(apiKey, input);
