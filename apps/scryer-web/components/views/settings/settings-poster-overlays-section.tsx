import * as React from "react";
import { Edit, Plus, RefreshCw, Trash2, Undo2 } from "lucide-react";

import { AddNewButton } from "@/components/common/add-new-button";
import { SettingsPosterOverlayTemplateEditor } from "@/components/views/settings/settings-poster-overlay-template-editor";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableActionsCell,
  TableActionsHead,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  PosterOverlayOverview,
  PosterOverlayPreviewFacet,
  PosterOverlayPreviewState,
  PosterOverlaySample,
  PosterOverlayTemplateDraft,
  PosterOverlayTemplateValidation,
} from "@/lib/types/poster-overlays";
import { selectorId } from "@/lib/utils/dom-ids";

/** Radix Select forbids an empty value, so the built-in template needs a token. */
export const BUILTIN_TEMPLATE_VALUE = "__builtin__";

const PANEL_CLASS =
  "overflow-hidden rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] shadow-[0_10px_24px_rgba(0,0,0,0.16)]";
const PANEL_HEADER_CLASS =
  "flex flex-wrap items-center justify-between gap-2 border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,rgba(255,255,255,0.035),rgba(255,255,255,0))] px-4 py-3";
const PANEL_TITLE_CLASS = "text-[15px] font-semibold text-[var(--scry-ink2)]";
const PANEL_BODY_CLASS = "p-4 sm:p-5";
const MUTED_TEXT_CLASS = "text-[var(--scry-muted3)]";

type SettingsPosterOverlaysSectionProps = {
  overview: PosterOverlayOverview | null;
  loading: boolean;
  loadError: boolean;
  busy: boolean;
  canManage: boolean;
  onSetLibrary: (libraryId: string, enabled: boolean, templateId: string | null) => void;
  settingsDraft: { parallelism: string; reconcileHours: string };
  setSettingsDraft: React.Dispatch<
    React.SetStateAction<{ parallelism: string; reconcileHours: string }>
  >;
  onSaveSettings: (event: React.FormEvent<HTMLFormElement>) => void;
  onRebuild: () => void;
  onRequestRevertAll: () => void;
  templateDraft: PosterOverlayTemplateDraft | null;
  setTemplateDraft: React.Dispatch<React.SetStateAction<PosterOverlayTemplateDraft | null>>;
  onStartTemplate: () => void;
  onEditTemplate: (templateId: string) => void;
  onRequestDeleteTemplate: (templateId: string) => void;
  onSaveTemplate: (event: React.FormEvent<HTMLFormElement>) => void;
  onValidateTemplate: () => void;
  validation: PosterOverlayTemplateValidation | null;
  preview: PosterOverlayPreviewState;
  previewFacet: PosterOverlayPreviewFacet;
  onPreviewFacetChange: (facet: PosterOverlayPreviewFacet) => void;
  onShufflePreview: () => void;
  sample: PosterOverlaySample;
  setSample: React.Dispatch<React.SetStateAction<PosterOverlaySample>>;
};

export function SettingsPosterOverlaysSection({
  overview,
  loading,
  loadError,
  busy,
  canManage,
  onSetLibrary,
  settingsDraft,
  setSettingsDraft,
  onSaveSettings,
  onRebuild,
  onRequestRevertAll,
  templateDraft,
  setTemplateDraft,
  onStartTemplate,
  onEditTemplate,
  onRequestDeleteTemplate,
  onSaveTemplate,
  onValidateTemplate,
  validation,
  preview,
  previewFacet,
  onPreviewFacetChange,
  onShufflePreview,
  sample,
  setSample,
}: SettingsPosterOverlaysSectionProps) {
  const t = useTranslate();

  if (loading) {
    return <p className={MUTED_TEXT_CLASS}>{t("label.loading")}</p>;
  }
  if (loadError || !overview) {
    return (
      <p id="settings-poster-overlays-load-error" className="text-destructive">
        {t("settings.posterOverlays.loadError")}
      </p>
    );
  }

  const { counts, libraries, templates } = overview;
  const disabled = busy || !canManage;

  return (
    <div id="settings-poster-overlays-section" className="space-y-5 text-sm">
      <p className={MUTED_TEXT_CLASS}>{t("settings.posterOverlays.description")}</p>

      <section className={PANEL_CLASS}>
        <div className={PANEL_HEADER_CLASS}>
          <h3 className={PANEL_TITLE_CLASS}>{t("settings.posterOverlays.statusTitle")}</h3>
          <div className="flex flex-wrap gap-2">
            <Button
              id="settings-poster-overlays-rebuild"
              variant="secondary"
              size="sm"
              disabled={disabled}
              onClick={onRebuild}
            >
              <RefreshCw className="size-4" />
              {t("settings.posterOverlays.rebuild")}
            </Button>
            <Button
              id="settings-poster-overlays-revert-all"
              variant="destructive"
              size="sm"
              disabled={disabled}
              onClick={onRequestRevertAll}
            >
              <Undo2 className="size-4" />
              {t("settings.posterOverlays.revertAll")}
            </Button>
          </div>
        </div>
        <div className={PANEL_BODY_CLASS}>
          <p id="settings-poster-overlays-counts">
            {t("settings.posterOverlays.counts", {
              rendered: counts.rendered,
              total: counts.enabledTitles,
            })}
            {counts.failed > 0 ? (
              <span className="ml-2 text-destructive">
                {t("settings.posterOverlays.failedCount", { count: counts.failed })}
              </span>
            ) : null}
            {counts.noArtwork > 0 ? (
              <span id="settings-poster-overlays-no-artwork" className={`ml-2 ${MUTED_TEXT_CLASS}`}>
                {t("settings.posterOverlays.noArtworkCount", { count: counts.noArtwork })}
              </span>
            ) : null}
          </p>
        </div>
      </section>

      <section className={PANEL_CLASS}>
        <div className={PANEL_HEADER_CLASS}>
          <h3 className={PANEL_TITLE_CLASS}>{t("settings.posterOverlays.librariesTitle")}</h3>
        </div>
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t("settings.posterOverlays.library")}</TableHead>
              <TableHead>{t("settings.posterOverlays.template")}</TableHead>
              <TableHead className="w-[1%] whitespace-nowrap">
                {t("settings.posterOverlays.enabled")}
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {libraries.map((library) => (
              <TableRow
                key={library.libraryId}
                id={selectorId("settings-poster-overlays-library", library.libraryId)}
              >
                <TableCell>
                  <span className="font-medium text-[var(--scry-ink2)]">{library.libraryName}</span>
                  <span className={`ml-2 ${MUTED_TEXT_CLASS}`}>{library.facet}</span>
                </TableCell>
                <TableCell>
                  <Select
                    value={library.templateId ?? BUILTIN_TEMPLATE_VALUE}
                    disabled={disabled}
                    onValueChange={(value) =>
                      onSetLibrary(
                        library.libraryId,
                        library.enabled,
                        value === BUILTIN_TEMPLATE_VALUE ? null : value,
                      )
                    }
                  >
                    <SelectTrigger
                      id={selectorId("settings-poster-overlays-template", library.libraryId)}
                      className="w-full max-w-[260px]"
                    >
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value={BUILTIN_TEMPLATE_VALUE}>
                        {t("settings.posterOverlays.builtinTemplate")}
                      </SelectItem>
                      {templates.map((template) => (
                        <SelectItem key={template.id} value={template.id}>
                          {template.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </TableCell>
                <TableCell>
                  <Switch
                    id={selectorId("settings-poster-overlays-enabled", library.libraryId)}
                    aria-label={t("settings.posterOverlays.enableFor", {
                      library: library.libraryName,
                    })}
                    checked={library.enabled}
                    disabled={disabled}
                    onCheckedChange={(enabled) =>
                      onSetLibrary(library.libraryId, enabled, library.templateId)
                    }
                  />
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </section>

      <section className={PANEL_CLASS}>
        <div className={PANEL_HEADER_CLASS}>
          <h3 className={PANEL_TITLE_CLASS}>{t("settings.posterOverlays.renderingTitle")}</h3>
        </div>
        <form
          id="settings-poster-overlays-settings-form"
          className={`${PANEL_BODY_CLASS} grid gap-4 sm:grid-cols-[1fr_1fr_auto] sm:items-end`}
          onSubmit={onSaveSettings}
        >
          <div className="space-y-1.5">
            <Label htmlFor="settings-poster-overlays-parallelism">
              {t("settings.posterOverlays.parallelism")}
            </Label>
            <Input
              id="settings-poster-overlays-parallelism"
              type="number"
              min={1}
              max={16}
              value={settingsDraft.parallelism}
              disabled={disabled}
              onChange={(event) =>
                setSettingsDraft((draft) => ({ ...draft, parallelism: event.target.value }))
              }
            />
            <p className={`text-xs ${MUTED_TEXT_CLASS}`}>
              {t("settings.posterOverlays.parallelismHelp")}
            </p>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="settings-poster-overlays-reconcile">
              {t("settings.posterOverlays.reconcileHours")}
            </Label>
            <Input
              id="settings-poster-overlays-reconcile"
              type="number"
              min={0.25}
              step={0.25}
              value={settingsDraft.reconcileHours}
              disabled={disabled}
              onChange={(event) =>
                setSettingsDraft((draft) => ({ ...draft, reconcileHours: event.target.value }))
              }
            />
            <p className={`text-xs ${MUTED_TEXT_CLASS}`}>
              {t("settings.posterOverlays.reconcileHelp")}
            </p>
          </div>
          <Button id="settings-poster-overlays-settings-save" type="submit" disabled={disabled}>
            {t("label.save")}
          </Button>
        </form>
      </section>

      <section className={PANEL_CLASS}>
        <div className={PANEL_HEADER_CLASS}>
          <h3 className={PANEL_TITLE_CLASS}>{t("settings.posterOverlays.templatesTitle")}</h3>
          {canManage && !templateDraft ? (
            <AddNewButton
              id="settings-poster-overlays-template-new"
              icon={Plus}
              label={t("settings.posterOverlays.newTemplate")}
              onClick={onStartTemplate}
            />
          ) : null}
        </div>
        {templates.length > 0 ? (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("settings.posterOverlays.templateName")}</TableHead>
                <TableActionsHead />
              </TableRow>
            </TableHeader>
            <TableBody>
              {templates.map((template) => (
                <TableRow
                  key={template.id}
                  id={selectorId("settings-poster-overlays-template-row", template.id)}
                >
                  <TableCell>{template.name}</TableCell>
                  <TableActionsCell>
                    <IconButton
                      id={selectorId("settings-poster-overlays-template-edit", template.id)}
                      label={t("label.edit")}
                      disabled={disabled}
                      onClick={() => onEditTemplate(template.id)}
                    >
                      <Edit className="size-4" />
                    </IconButton>
                    <IconButton
                      id={selectorId("settings-poster-overlays-template-delete", template.id)}
                      label={t("label.delete")}
                      disabled={disabled}
                      onClick={() => onRequestDeleteTemplate(template.id)}
                    >
                      <Trash2 className="size-4" />
                    </IconButton>
                  </TableActionsCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        ) : (
          <p className={`${PANEL_BODY_CLASS} ${MUTED_TEXT_CLASS}`}>
            {t("settings.posterOverlays.noTemplates")}
          </p>
        )}

        {templateDraft ? (
          <SettingsPosterOverlayTemplateEditor
            overview={overview}
            draft={templateDraft}
            setDraft={setTemplateDraft}
            busy={busy}
            onSave={onSaveTemplate}
            onValidate={onValidateTemplate}
            validation={validation}
            preview={preview}
            previewFacet={previewFacet}
            onPreviewFacetChange={onPreviewFacetChange}
            onShufflePreview={onShufflePreview}
            sample={sample}
            setSample={setSample}
          />
        ) : null}
      </section>
    </div>
  );
}
