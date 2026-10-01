import * as React from "react";
import { useClient } from "urql";

import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import {
  deletePosterOverlayTemplateMutation,
  rebuildPosterOverlaysMutation,
  revertAllPosterOverlaysMutation,
  savePosterOverlayTemplateMutation,
  setPosterOverlayLibraryMutation,
  updatePosterOverlaySettingsMutation,
} from "@/lib/graphql/mutations";
import {
  posterOverlaysQuery,
  previewPosterOverlayTemplateQuery,
  validatePosterOverlayTemplateQuery,
} from "@/lib/graphql/queries";
import type {
  PosterOverlayOverview,
  PosterOverlayPreviewState,
  PosterOverlaySample,
  PosterOverlayTemplateDraft,
  PosterOverlayTemplateValidation,
} from "@/lib/types/poster-overlays";

/**
 * Network side of Settings > Poster overlays. The backend owns every rule;
 * this hook loads the overview, sends each change, and reloads so the page
 * always shows what the server stored.
 */
export function usePosterOverlays() {
  const client = useClient();
  const t = useTranslate();
  const showStatus = useGlobalStatus();
  const [overview, setOverview] = React.useState<PosterOverlayOverview | null>(null);
  const [loading, setLoading] = React.useState(true);
  const [loadError, setLoadError] = React.useState(false);
  const [busy, setBusy] = React.useState(false);

  const reload = React.useCallback(async () => {
    const result = await client
      .query(posterOverlaysQuery, {}, { requestPolicy: "network-only" })
      .toPromise();
    if (result.error || !result.data?.posterOverlays) {
      setLoadError(true);
    } else {
      setOverview(result.data.posterOverlays as PosterOverlayOverview);
      setLoadError(false);
    }
    setLoading(false);
  }, [client]);

  React.useEffect(() => {
    void reload();
  }, [reload]);

  /** Runs a mutation, reports failure, and reloads on success. */
  const run = React.useCallback(
    async (
      document: string,
      variables: Record<string, unknown>,
      failureKey: string,
    ): Promise<Record<string, unknown> | null> => {
      setBusy(true);
      try {
        const result = await client.mutation(document, variables).toPromise();
        if (result.error || !result.data) {
          showStatus(result.error?.message || t(failureKey));
          return null;
        }
        await reload();
        return result.data as Record<string, unknown>;
      } finally {
        setBusy(false);
      }
    },
    [client, reload, showStatus, t],
  );

  const setLibrary = React.useCallback(
    (libraryId: string, enabled: boolean, templateId: string | null) =>
      run(
        setPosterOverlayLibraryMutation,
        { input: { libraryId, enabled, templateId } },
        "settings.posterOverlays.saveError",
      ),
    [run],
  );

  const saveSettings = React.useCallback(
    async (parallelism: number, reconcileIntervalSeconds: number) => {
      const saved = await run(
        updatePosterOverlaySettingsMutation,
        { input: { parallelism, reconcileIntervalSeconds } },
        "settings.posterOverlays.saveError",
      );
      if (saved) {
        showStatus(t("settings.posterOverlays.settingsSaved"));
      }
      return saved !== null;
    },
    [run, showStatus, t],
  );

  const saveTemplate = React.useCallback(
    async (draft: PosterOverlayTemplateDraft) => {
      const saved = await run(
        savePosterOverlayTemplateMutation,
        { input: { id: draft.id || null, name: draft.name, svg: draft.svg } },
        "settings.posterOverlays.saveError",
      );
      return saved !== null;
    },
    [run],
  );

  const deleteTemplate = React.useCallback(
    async (id: string) =>
      (await run(
        deletePosterOverlayTemplateMutation,
        { id },
        "settings.posterOverlays.saveError",
      )) !== null,
    [run],
  );

  const rebuild = React.useCallback(async () => {
    const result = await run(
      rebuildPosterOverlaysMutation,
      {},
      "settings.posterOverlays.saveError",
    );
    if (result) {
      showStatus(t("settings.posterOverlays.rebuildQueued"));
    }
  }, [run, showStatus, t]);

  const revertAll = React.useCallback(async () => {
    const result = await run(
      revertAllPosterOverlaysMutation,
      {},
      "settings.posterOverlays.revertError",
    );
    if (result) {
      showStatus(
        t("settings.posterOverlays.reverted", {
          count: Number(result.revertAllPosterOverlays ?? 0),
        }),
      );
    }
    return result !== null;
  }, [run, showStatus, t]);

  const validateTemplate = React.useCallback(
    async (svg: string): Promise<PosterOverlayTemplateValidation> => {
      const result = await client
        .query(validatePosterOverlayTemplateQuery, { svg }, { requestPolicy: "network-only" })
        .toPromise();
      if (result.error || !result.data?.validatePosterOverlayTemplate) {
        return {
          valid: false,
          error: result.error?.message || t("settings.posterOverlays.validateError"),
        };
      }
      return result.data.validatePosterOverlayTemplate as PosterOverlayTemplateValidation;
    },
    [client, t],
  );

  return {
    overview,
    loading,
    loadError,
    busy,
    reload,
    setLibrary,
    saveSettings,
    saveTemplate,
    deleteTemplate,
    rebuild,
    revertAll,
    validateTemplate,
  };
}

/** Pause after the last edit before rendering, so typing does not queue renders. */
const PREVIEW_DEBOUNCE_MS = 250;

/**
 * Live preview of a draft template. Renders on the server after edits settle;
 * a response that arrives after a newer request was sent is dropped, so the
 * preview always matches the latest draft. The last good image stays on
 * screen while a new one renders or when the draft is invalid.
 */
export function usePosterOverlayPreview(
  svg: string | null,
  sample: PosterOverlaySample,
): PosterOverlayPreviewState {
  const client = useClient();
  const t = useTranslate();
  const [state, setState] = React.useState<PosterOverlayPreviewState>({
    image: null,
    libraryPoster: false,
    error: null,
    loading: false,
  });
  const latestRequest = React.useRef(0);

  React.useEffect(() => {
    if (svg === null) {
      return;
    }
    const request = ++latestRequest.current;
    setState((current) => ({ ...current, loading: true }));
    const timer = window.setTimeout(() => {
      void client
        .query(
          previewPosterOverlayTemplateQuery,
          { input: { svg, ...sample } },
          { requestPolicy: "network-only" },
        )
        .toPromise()
        .then((result) => {
          if (request !== latestRequest.current) {
            return;
          }
          const preview = result.data?.previewPosterOverlayTemplate as
            | { image: string | null; libraryPoster: boolean; error: string | null }
            | undefined;
          if (result.error || !preview) {
            setState((current) => ({
              ...current,
              loading: false,
              error: result.error?.message || t("settings.posterOverlays.previewError"),
            }));
            return;
          }
          setState((current) => ({
            image: preview.image ?? current.image,
            libraryPoster: preview.image ? preview.libraryPoster : current.libraryPoster,
            error: preview.error,
            loading: false,
          }));
        });
    }, PREVIEW_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [client, sample, svg, t]);

  return state;
}
