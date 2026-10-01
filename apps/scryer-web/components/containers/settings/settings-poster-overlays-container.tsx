import * as React from "react";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { SettingsPosterOverlaysSection } from "@/components/views/settings/settings-poster-overlays-section";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { usePosterOverlayPreview, usePosterOverlays } from "@/lib/hooks/use-poster-overlays";
import type {
  PosterOverlaySample,
  PosterOverlayTemplateDraft,
  PosterOverlayTemplateValidation,
} from "@/lib/types/poster-overlays";

const SECONDS_PER_HOUR = 3600;

const EMPTY_SAMPLE: PosterOverlaySample = {
  resolution: "",
  hdr: "",
  audio: "",
  audioChannels: "",
  edition: "",
};

type SettingsPosterOverlaysContainerProps = {
  /** Overlay configuration is catalog configuration. */
  canManageCatalogSettings: boolean;
};

export function SettingsPosterOverlaysContainer({
  canManageCatalogSettings,
}: SettingsPosterOverlaysContainerProps) {
  const t = useTranslate();
  const showStatus = useGlobalStatus();
  const overlays = usePosterOverlays();
  const { overview } = overlays;

  const [settingsDraft, setSettingsDraft] = React.useState({
    parallelism: "",
    reconcileHours: "",
  });
  const [templateDraft, setTemplateDraft] = React.useState<PosterOverlayTemplateDraft | null>(
    null,
  );
  const [validation, setValidation] = React.useState<PosterOverlayTemplateValidation | null>(
    null,
  );
  const [sample, setSample] = React.useState<PosterOverlaySample | null>(null);
  const [confirmRevert, setConfirmRevert] = React.useState(false);
  const [pendingDeleteId, setPendingDeleteId] = React.useState<string | null>(null);

  // Mirror stored settings into the form whenever the server copy changes.
  const storedParallelism = overview?.settings.parallelism;
  const storedInterval = overview?.settings.reconcileIntervalSeconds;
  React.useEffect(() => {
    if (storedParallelism === undefined || storedInterval === undefined) {
      return;
    }
    setSettingsDraft({
      parallelism: String(storedParallelism),
      reconcileHours: String(storedInterval / SECONDS_PER_HOUR),
    });
  }, [storedParallelism, storedInterval]);

  // Preview with the best value of every field, so every badge shows.
  const sampleOptions = overview?.sampleOptions;
  const defaultEdition = t("settings.posterOverlays.sampleEditionDefault");
  React.useEffect(() => {
    if (!sampleOptions) {
      return;
    }
    setSample(
      (current) =>
        current ?? {
          resolution: sampleOptions.resolutions[0]?.token ?? "",
          hdr: sampleOptions.hdr[0]?.token ?? "",
          audio: sampleOptions.audio[0]?.token ?? "",
          audioChannels: sampleOptions.audioChannels[0] ?? "",
          edition: defaultEdition,
        },
    );
  }, [defaultEdition, sampleOptions]);
  const activeSample = sample ?? EMPTY_SAMPLE;
  const preview = usePosterOverlayPreview(
    templateDraft && sample ? templateDraft.svg : null,
    activeSample,
  );

  // A template edit invalidates the last validation result.
  const draftSvg = templateDraft?.svg;
  React.useEffect(() => {
    setValidation(null);
  }, [draftSvg]);

  const saveSettings = React.useCallback(
    (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      const parallelism = Number.parseInt(settingsDraft.parallelism, 10);
      const hours = Number.parseFloat(settingsDraft.reconcileHours);
      if (!Number.isFinite(parallelism) || !Number.isFinite(hours)) {
        showStatus(t("settings.posterOverlays.settingsInvalid"));
        return;
      }
      void overlays.saveSettings(parallelism, Math.round(hours * SECONDS_PER_HOUR));
    },
    [overlays, settingsDraft, showStatus, t],
  );

  const startTemplate = React.useCallback(() => {
    setTemplateDraft({
      id: "",
      name: "",
      svg: overview?.builtinTemplate ?? "",
    });
  }, [overview?.builtinTemplate]);

  const editTemplate = React.useCallback(
    (templateId: string) => {
      const template = overview?.templates.find((candidate) => candidate.id === templateId);
      if (template) {
        setTemplateDraft({ id: template.id, name: template.name, svg: template.svg });
      }
    },
    [overview?.templates],
  );

  const saveTemplate = React.useCallback(
    async (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      if (!templateDraft) {
        return;
      }
      if (!templateDraft.name.trim()) {
        showStatus(t("settings.posterOverlays.templateNameRequired"));
        return;
      }
      if (await overlays.saveTemplate(templateDraft)) {
        setTemplateDraft(null);
      }
    },
    [overlays, showStatus, t, templateDraft],
  );

  const validateTemplate = React.useCallback(async () => {
    if (templateDraft) {
      setValidation(await overlays.validateTemplate(templateDraft.svg));
    }
  }, [overlays, templateDraft]);

  const pendingDeleteTemplate = React.useMemo(
    () =>
      pendingDeleteId
        ? (overview?.templates.find((template) => template.id === pendingDeleteId) ?? null)
        : null,
    [overview?.templates, pendingDeleteId],
  );

  return (
    <>
      <SettingsPosterOverlaysSection
        overview={overview}
        loading={overlays.loading}
        loadError={overlays.loadError}
        busy={overlays.busy}
        canManage={canManageCatalogSettings}
        onSetLibrary={(libraryId, enabled, templateId) =>
          void overlays.setLibrary(libraryId, enabled, templateId)
        }
        settingsDraft={settingsDraft}
        setSettingsDraft={setSettingsDraft}
        onSaveSettings={saveSettings}
        onRebuild={() => void overlays.rebuild()}
        onRequestRevertAll={() => setConfirmRevert(true)}
        templateDraft={templateDraft}
        setTemplateDraft={setTemplateDraft}
        onStartTemplate={startTemplate}
        onEditTemplate={editTemplate}
        onRequestDeleteTemplate={setPendingDeleteId}
        onSaveTemplate={(event) => void saveTemplate(event)}
        onValidateTemplate={() => void validateTemplate()}
        validation={validation}
        preview={preview}
        sample={activeSample}
        setSample={(update) =>
          setSample((current) => {
            const base = current ?? EMPTY_SAMPLE;
            return typeof update === "function" ? update(base) : update;
          })
        }
      />
      <ConfirmDialog
        open={confirmRevert}
        title={t("settings.posterOverlays.revertConfirmTitle")}
        description={t("settings.posterOverlays.revertConfirm")}
        confirmLabel={t("settings.posterOverlays.revertAll")}
        cancelLabel={t("label.cancel")}
        confirmButtonId="settings-poster-overlays-revert-confirm"
        isBusy={overlays.busy}
        onConfirm={async () => {
          if (await overlays.revertAll()) {
            setConfirmRevert(false);
          }
        }}
        onCancel={() => setConfirmRevert(false)}
      />
      <ConfirmDialog
        open={pendingDeleteTemplate !== null}
        title={t("settings.posterOverlays.deleteTemplateTitle")}
        description={t("settings.posterOverlays.deleteTemplateConfirm", {
          name: pendingDeleteTemplate?.name ?? "",
        })}
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        confirmButtonId="settings-poster-overlays-template-delete-confirm"
        isBusy={overlays.busy}
        onConfirm={async () => {
          if (pendingDeleteId && (await overlays.deleteTemplate(pendingDeleteId))) {
            if (templateDraft?.id === pendingDeleteId) {
              setTemplateDraft(null);
            }
          }
          setPendingDeleteId(null);
        }}
        onCancel={() => setPendingDeleteId(null)}
      />
    </>
  );
}
