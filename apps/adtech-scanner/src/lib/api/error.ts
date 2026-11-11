import type { CommandError, ErrorCode } from "@/bindings";

/** A `CommandError` from the backend, as a throwable. */
export class ApiError extends Error {
  readonly code: ErrorCode;

  constructor(code: ErrorCode, message: string) {
    super(message);
    this.name = "ApiError";
    this.code = code;
  }
}

function isCommandError(value: unknown): value is CommandError {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as CommandError).code === "string" &&
    typeof (value as CommandError).message === "string"
  );
}

export function toApiError(reason: unknown): ApiError {
  if (reason instanceof ApiError) return reason;
  if (isCommandError(reason)) return new ApiError(reason.code, reason.message);
  if (reason instanceof Error) return new ApiError("internal", reason.message);
  return new ApiError("internal", typeof reason === "string" ? reason : "Unexpected error");
}
