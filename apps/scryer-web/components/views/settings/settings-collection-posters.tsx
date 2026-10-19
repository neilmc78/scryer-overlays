import * as React from "react";
import { useClient } from "urql";

import { ChevronDown, ChevronRight } from "lucide-react";

import type { PlexCollectionList } from "@/components/views/lists/plex-collection-setting";
import { ProviderTile } from "@/components/views/lists/provider-tile";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import {
  setPlexCollectionPosterMutation,
  uploadPlexCollectionLogoMutation,
} from "@/lib/graphql/mutations";
import { plexCollectionListsQuery, previewPlexCollectionPosterQuery } from "@/lib/graphql/queries";

const PREVIEW_DELAY_MS = 400;
const MAX_LABEL = 60;
// Matches the server's limit on uploaded logos.
const MAX_LOGO_BYTES = 2 * 1024 * 1024;
const LOGO_TYPES = "image/png,image/jpeg,image/webp";

type LogoSource = "provider" | "custom" | "none";

type PosterDraft = {
  enabled: boolean;
  background: string;
  text: string;
  logoSource: LogoSource;
};

function draftOf(list: PlexCollectionList): PosterDraft {
  return {
    enabled: list.posterEnabled,
    background: list.posterBackground,
    text: list.posterText ?? "",
    logoSource: list.posterLogoSource,
  };
}

function posterInput(subscriptionId: string, draft: PosterDraft) {
  return {
    subscriptionId,
    enabled: draft.enabled,
    background: draft.background,
    text: draft.text.trim() || null,
    showLogo: draft.logoSource === "provider",
    logoSource: draft.logoSource,
  };
}

async function fileBase64(file: File): Promise<string> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  let binary = "";
  const chunk = 0x8000;
  for (let index = 0; index < bytes.length; index += chunk) {
    binary += String.fromCharCode(...bytes.subarray(index, index + chunk));
  }
  return btoa(binary);
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
  // Bumped after an upload so the preview draws the new image.
  const [logoVersion, setLogoVersion] = React.useState(0);
  const fileInput = React.useRef<HTMLInputElement>(null);
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
  }, [client, list.subscriptionId, draft, logoVersion]);

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

  const upload = async (file: File | undefined) => {
    if (!file) {
      return;
    }
    if (file.size > MAX_LOGO_BYTES) {
      showStatus(t("settings.collectionPosters.logoTooLarge"));
      return;
    }
    setBusy(true);
    const result = await client
      .mutation(uploadPlexCollectionLogoMutation, {
        subscriptionId: list.subscriptionId,
        imageBase64: await fileBase64(file),
      })
      .toPromise();
    if (result.error) {
      showStatus(result.error.message || t("settings.collectionPosters.logoUploadFailed"));
    } else {
      showStatus(t("settings.collectionPosters.logoUploaded"));
      update({ logoSource: "custom" });
      setLogoVersion((version) => version + 1);
      await onSaved();
    }
    setBusy(false);
  };

  return (
    <div id={idBase} className="flex flex-col gap-4 py-2 sm:flex-row">
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
        <div className="space-y-1">
          <Label>{t("settings.collectionPosters.logoSource")}</Label>
          <RadioGroup
            id={`${idBase}-logo`}
            className="flex flex-wrap gap-4"
            value={draft.logoSource}
            disabled={busy}
            onValueChange={(value) => update({ logoSource: value as LogoSource })}
          >
            {(["provider", "custom", "none"] as const).map((source) => (
              <label
                key={source}
                htmlFor={`${idBase}-logo-${source}`}
                className="flex items-center gap-2 text-[13px] text-[var(--scry-ink2)]"
              >
                <RadioGroupItem
                  id={`${idBase}-logo-${source}`}
                  value={source}
                  disabled={source === "provider" && !list.posterLogoAvailable}
                />
                {t(`settings.collectionPosters.logoSource.${source}`)}
              </label>
            ))}
          </RadioGroup>
          {!list.posterLogoAvailable ? (
            <p className="text-[12px] text-[var(--scry-muted)]">
              {t("settings.collectionPosters.noLogo")}
            </p>
          ) : null}
        </div>
        {draft.logoSource === "custom" ? (
          <div className="flex flex-wrap items-center gap-3">
            <input
              ref={fileInput}
              id={`${idBase}-logo-file`}
              type="file"
              accept={LOGO_TYPES}
              className="hidden"
              onChange={(event) => {
                const file = event.target.files?.[0];
                event.target.value = "";
                void upload(file);
              }}
            />
            <Button
              id={`${idBase}-logo-upload`}
              type="button"
              size="xs"
              variant="secondary"
              disabled={busy}
              onClick={() => fileInput.current?.click()}
            >
              {list.posterCustomLogo
                ? t("settings.collectionPosters.logoReplace")
                : t("settings.collectionPosters.logoUpload")}
            </Button>
            <span className="text-[12px] text-[var(--scry-muted)]">
              {list.posterCustomLogo
                ? t("settings.collectionPosters.logoHelp")
                : t("settings.collectionPosters.logoMissing")}
            </span>
          </div>
        ) : null}
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
    </div>
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
  // One list's poster editor is open at a time; its preview renders only then.
  const [openId, setOpenId] = React.useState<string | null>(null);

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
          <Table
            id="settings-collection-posters-table"
            wrapperClassName="rounded-[12px] border border-[var(--scry-border3)]"
          >
            <TableHeader>
              <TableRow>
                <TableHead>{t("settings.collectionPosters.list")}</TableHead>
                <TableHead className="w-[1%]">{t("settings.collectionPosters.poster")}</TableHead>
                <TableHead className="hidden w-[1%] sm:table-cell">
                  {t("settings.collectionPosters.background")}
                </TableHead>
                <TableHead className="hidden md:table-cell">
                  {t("settings.collectionPosters.label")}
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {lists.map((list) => {
                const open = openId === list.subscriptionId;
                const Chevron = open ? ChevronDown : ChevronRight;
                return (
                  <React.Fragment key={list.subscriptionId}>
                    <TableRow
                      id={`collection-poster-row-${list.subscriptionId}`}
                      className="cursor-pointer hover:bg-[var(--scry-hover)]"
                      aria-expanded={open}
                      onClick={() => setOpenId(open ? null : list.subscriptionId)}
                    >
                      <TableCell>
                        <div className="flex min-w-0 items-center gap-3">
                          <Chevron className="h-4 w-4 shrink-0 text-[var(--scry-muted)]" />
                          <ProviderTile
                            provider={{
                              providerType: list.provider,
                              name: list.provider,
                              tile: null,
                            }}
                            size="sm"
                          />
                          <span className="truncate text-[14px] font-semibold text-[var(--scry-ink)]">
                            {list.name}
                          </span>
                        </div>
                      </TableCell>
                      <TableCell>
                        <Badge tone={list.posterEnabled ? "positive" : "neutral"}>
                          {list.posterEnabled
                            ? t("settings.collectionPosters.on")
                            : t("settings.collectionPosters.off")}
                        </Badge>
                      </TableCell>
                      <TableCell className="hidden sm:table-cell">
                        <span className="flex items-center gap-2">
                          <span
                            aria-hidden="true"
                            className="h-4 w-6 rounded border border-[var(--scry-border2)]"
                            style={{ backgroundColor: list.posterBackground }}
                          />
                          <span className="font-mono text-[12px] text-[var(--scry-muted)]">
                            {list.posterBackground}
                          </span>
                        </span>
                      </TableCell>
                      <TableCell className="hidden truncate text-[13px] text-[var(--scry-ink2)] md:table-cell">
                        {list.posterText ?? list.name}
                      </TableCell>
                    </TableRow>
                    {open ? (
                      <TableRow className="hover:bg-transparent">
                        <TableCell colSpan={4}>
                          <CollectionPosterRow list={list} onSaved={reload} />
                        </TableCell>
                      </TableRow>
                    ) : null}
                  </React.Fragment>
                );
              })}
            </TableBody>
          </Table>
        )}
      </div>
    </section>
  );
}
