import * as React from "react";
import { useClient } from "urql";

import type { PlexCollectionList } from "@/components/views/lists/plex-collection-setting";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { setPlexCollectionPosterMutation } from "@/lib/graphql/mutations";
import { plexCollectionListsQuery, previewPlexCollectionPosterQuery } from "@/lib/graphql/queries";

const PREVIEW_DELAY_MS = 400;
const MAX_LABEL = 60;

type PosterDraft = {
  enabled: boolean;
  background: string;
  text: string;
  showLogo: boolean;
};

function draftOf(list: PlexCollectionList): PosterDraft {
  return {
    enabled: list.posterEnabled,
    background: list.posterBackground,
    text: list.posterText ?? "",
    showLogo: list.posterLogo,
  };
}

function posterInput(subscriptionId: string, draft: PosterDraft) {
  return {
    subscriptionId,
    enabled: draft.enabled,
    background: draft.background,
    text: draft.text.trim() || null,
    showLogo: draft.showLogo,
  };
}

type PosterRowProps = {
  list: PlexCollectionList;
  onSaved: () => Promise<void>;
};

function CollectionPosterRow({ list, onSaved }: PosterRowProps) {
  const client = useClient();
  const t = useTranslate();
  const showStatus = useGlobalStatus();
  const [draft, setDraft] = React.useState<PosterDraft>(() => draftOf(list));
  const [preview, setPreview] = React.useState<string | null>(null);
  const [previewError, setPreviewError] = React.useState<string | null>(null);
  const [busy, setBusy] = React.useState(false);
  const idBase = `collection-poster-${list.subscriptionId}`;

  React.useEffect(() => {
    let cancelled = false;
    const timer = window.setTimeout(async () => {
      const result = await client
        .query(
          previewPlexCollectionPosterQuery,
          { input: posterInput(list.subscriptionId, draft) },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (cancelled) {
        return;
      }
      const payload = result.data?.previewPlexCollectionPoster as
        { image: string | null; error: string | null } | undefined;
      setPreview(payload?.image ?? null);
      setPreviewError(payload?.error ?? result.error?.message ?? null);
    }, PREVIEW_DELAY_MS);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [client, list.subscriptionId, draft]);

  const update = (change: Partial<PosterDraft>) =>
    setDraft((current) => ({ ...current, ...change }));

  const save = async () => {
    setBusy(true);
    const result = await client
      .mutation(setPlexCollectionPosterMutation, { input: posterInput(list.subscriptionId, draft) })
      .toPromise();
    showStatus(
      result.error
        ? result.error.message || t("settings.collectionPosters.saveFailed")
        : t("settings.collectionPosters.saved"),
    );
    await onSaved();
    setBusy(false);
  };

  return (
    <li
      id={idBase}
      className="flex flex-col gap-4 border-b border-[var(--scry-border3)] py-4 last:border-b-0 sm:flex-row"
    >
      <div className="aspect-[2/3] w-32 shrink-0 overflow-hidden rounded-md bg-[var(--scry-inset)]">
        {preview ? (
          <img
            src={preview}
            alt={t("settings.collectionPosters.previewAlt", { name: list.name })}
            className="h-full w-full object-cover"
          />
        ) : null}
      </div>
      <div className="min-w-0 flex-1 space-y-3">
        <div className="flex items-center justify-between gap-3">
          <span className="truncate text-[14px] font-medium text-[var(--scry-ink)]">
            {list.name}
          </span>
          <Switch
            id={`${idBase}-enabled`}
            aria-label={t("settings.collectionPosters.enabled")}
            checked={draft.enabled}
            disabled={busy}
            onCheckedChange={(enabled) => update({ enabled })}
          />
        </div>
        <div className="flex flex-wrap items-end gap-3">
          <div className="space-y-1">
            <Label htmlFor={`${idBase}-background`}>
              {t("settings.collectionPosters.background")}
            </Label>
            <div className="flex items-center gap-2">
              <input
                id={`${idBase}-background`}
                type="color"
                value={draft.background}
                disabled={busy}
                onChange={(event) => update({ background: event.target.value.toUpperCase() })}
                className="h-9 w-14 cursor-pointer rounded border border-[var(--scry-border2)] bg-transparent"
              />
              <span className="font-mono text-[12px] text-[var(--scry-muted)]">
                {draft.background}
              </span>
            </div>
          </div>
          <div className="min-w-[12rem] flex-1 space-y-1">
            <Label htmlFor={`${idBase}-label`}>{t("settings.collectionPosters.label")}</Label>
            <Input
              id={`${idBase}-label`}
              value={draft.text}
              maxLength={MAX_LABEL}
              placeholder={list.name}
              disabled={busy}
              onChange={(event) => update({ text: event.target.value })}
            />
          </div>
        </div>
        <p className="text-[12px] text-[var(--scry-muted)]">
          {t("settings.collectionPosters.labelHelp")}
        </p>
        {list.posterLogoAvailable ? (
          <label className="flex items-center gap-2 text-[13px] text-[var(--scry-ink2)]">
            <Checkbox
              id={`${idBase}-logo`}
              checked={draft.showLogo}
              disabled={busy}
              onCheckedChange={(checked) => update({ showLogo: checked === true })}
            />
            {t("settings.collectionPosters.logo")}
          </label>
        ) : (
          <p className="text-[12px] text-[var(--scry-muted)]">
            {t("settings.collectionPosters.noLogo")}
          </p>
        )}
        {previewError ? <p className="text-[12.5px] text-destructive">{previewError}</p> : null}
        <Button
          id={`${idBase}-save`}
          type="button"
          size="xs"
          onClick={() => void save()}
          disabled={busy}
        >
          {t("settings.collectionPosters.save")}
        </Button>
      </div>
    </li>
  );
}

type SettingsCollectionPostersProps = {
  panelClass: string;
  headerClass: string;
  titleClass: string;
  bodyClass: string;
  mutedClass: string;
};

/** Collection posters for mirrored lists. Loads its own data. */
export function SettingsCollectionPosters({
  panelClass,
  headerClass,
  titleClass,
  bodyClass,
  mutedClass,
}: SettingsCollectionPostersProps) {
  const client = useClient();
  const t = useTranslate();
  const [lists, setLists] = React.useState<PlexCollectionList[] | null>(null);

  const reload = React.useCallback(async () => {
    const result = await client
      .query(plexCollectionListsQuery, {}, { requestPolicy: "network-only" })
      .toPromise();
    if (result.error) {
      setLists(null);
      return;
    }
    setLists(
      ((result.data?.plexCollectionLists ?? []) as PlexCollectionList[]).filter(
        (list) => list.enabled,
      ),
    );
  }, [client]);

  React.useEffect(() => {
    void reload();
  }, [reload]);

  if (lists === null) {
    return null;
  }

  return (
    <section id="settings-collection-posters" className={panelClass}>
      <div className={headerClass}>
        <h3 className={titleClass}>{t("settings.collectionPosters.title")}</h3>
      </div>
      <div className={`${bodyClass} space-y-2`}>
        <p className={mutedClass}>{t("settings.collectionPosters.help")}</p>
        {lists.length === 0 ? (
          <p className={mutedClass}>{t("settings.collectionPosters.empty")}</p>
        ) : (
          <ul>
            {lists.map((list) => (
              <CollectionPosterRow key={list.subscriptionId} list={list} onSaved={reload} />
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}
