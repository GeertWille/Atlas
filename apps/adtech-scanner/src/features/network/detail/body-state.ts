import type { BodyRef, BodySide } from "@/bindings";
import { formatBytes } from "@/shared/lib/format";

export type BodyNotice = {
  title: string;
  /** Why, in a sentence. */
  detail: string | null;
  /** The body is gone because of the storage limit: the notice links to the storage settings. */
  storage: boolean;
};

/** What a MIME type is, in a word, for "Body not kept (image, 48 KB)". */
export function mediaWord(mime: string | null): string {
  const type = (mime ?? "").toLowerCase();
  if (type.startsWith("image/")) return "image";
  if (type.startsWith("video/")) return "video";
  if (type.startsWith("audio/")) return "audio";
  if (type.startsWith("font/") || type.includes("woff")) return "font";
  return "binary data";
}

/**
 * What to say in place of a body that cannot be read, or above one that is incomplete. `null` for a
 * body that is stored whole. Every state has its own sentence: "not kept" is never left unexplained.
 */
export function bodyNotice(body: BodyRef, side: BodySide): BodyNotice | null {
  const size = formatBytes(body.bytes);
  const was = body.bytes > 0 ? ` (was ${size})` : "";
  switch (body.state) {
    case "kept":
      return null;
    case "truncated":
      return {
        title: `First ${formatBytes(body.storedBytes)} of ${size} kept`,
        detail: "The rest was over the size limit for one body and was not stored.",
        storage: false,
      };
    case "empty":
      return { title: side === "response" ? "No content" : "No request body", detail: null, storage: false };
    case "notKeptType":
      return {
        title: `Body not kept (${mediaWord(body.mime)}, ${size})`,
        detail: "Images, video, audio and fonts are recorded as metadata only.",
        storage: false,
      };
    case "scanLimit":
      return {
        title: `Body not kept${was}`,
        detail: "The scan had reached its cap for stored bodies when this one arrived.",
        storage: false,
      };
    case "dropped":
      return {
        title: `Body not kept${was}`,
        detail: "The archive could not keep up with the page at that moment and left this body out.",
        storage: false,
      };
    case "unavailable":
      return {
        title: "The browser could not provide this body",
        detail:
          "That is the case for failed and cancelled loads, redirects and preflights, and for bodies Chrome had already discarded.",
        storage: false,
      };
    case "pending":
      return { title: "No response before the visit ended", detail: null, storage: false };
    case "evicted":
      return {
        title: `Body removed to stay under the storage limit${was}`,
        detail: "The request itself is still listed; its headers and bodies are gone.",
        storage: true,
      };
  }
}
