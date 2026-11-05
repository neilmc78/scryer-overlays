import type { Translate } from "@/components/root/types";
import type { PosterPushServerKind } from "@/lib/types/poster-overlays";
import {
  graphQlErrorExtension,
  userFacingGraphQlErrorMessage,
} from "../graphql/error-message.ts";

/**
 * Poster overlay refusals the server names with `extensions.reason`
 * (`crates/scryer-application/src/overlays/reasons.rs`). Each is said in the
 * viewer's language; any other failure shows the server's sentence.
 */
export const OVERLAY_REFUSAL_KEYS: Readonly<Record<string, string>> = {
  OVERLAY_PUSH_SERVER_KIND_UNKNOWN:
    "settings.posterOverlays.refusal.serverKindUnknown",
  OVERLAY_PUSH_ADDRESS_INVALID: "settings.posterOverlays.refusal.addressInvalid",
  OVERLAY_PUSH_ADDRESS_REQUIRED:
    "settings.posterOverlays.refusal.addressRequired",
  OVERLAY_PUSH_CONNECTION_MISSING:
    "settings.posterOverlays.refusal.connectionMissing",
};

/** The translation key for an overlay refusal, if this client knows it. */
export function overlayRefusalKey(error: unknown): string | null {
  const reason = graphQlErrorExtension(error, "VALIDATION_ERROR", "reason");
  return reason !== null && Object.hasOwn(OVERLAY_REFUSAL_KEYS, reason)
    ? OVERLAY_REFUSAL_KEYS[reason]
    : null;
}

/**
 * What to show for a failed overlay request: the translated reason when the
 * server named one this client knows, else the server's own sentence, else
 * `fallback`.
 */
export function overlayErrorMessage(
  error: unknown,
  t: Translate,
  fallback: string,
): string {
  const key = overlayRefusalKey(error);
  return key ? t(key) : userFacingGraphQlErrorMessage(error, fallback, t);
}

/** The push server's product name; not translated. */
export function posterPushServerName(kind: PosterPushServerKind): string {
  return kind === "jellyfin" ? "Jellyfin" : "Silo";
}
