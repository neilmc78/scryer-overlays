import * as React from "react";
import { ChevronDown, Layers, Loader2, Plus, Trash2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { IconButton } from "@/components/ui/icon-button";
import { Input } from "@/components/ui/input";
import { MultiSelectDropdown } from "@/components/ui/multi-select-dropdown";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  PosterOverlayOverview,
  PosterOverlayPreviewState,
  PosterOverlaySample,
  PosterOverlaySampleOptions,
  PosterOverlayTemplateDraft,
  PosterOverlayTemplateValidation,
} from "@/lib/types/poster-overlays";
import { selectorId } from "@/lib/utils/dom-ids";
import {
  BADGE_KINDS,
  CONDITION_FIELDS,
  OVERLAY_CANVAS_HEIGHT,
  OVERLAY_CANVAS_WIDTH,
  badgeKind,
  clampElement,
  editionToken,
  formatBadgeCondition,
  newElement,
  parseBadgeCondition,
  parseOverlay,
  serializeOverlay,
  type BadgeCondition,
  type ConditionField,
  type OverlayBadgeKind,
  type OverlayElement,
  type OverlayTextAlign,
} from "@/lib/utils/poster-overlay-builder";

/** Radix Select forbids an empty value, so "no value" needs a token. */
const NONE_VALUE = "__none__";

const MUTED_TEXT_CLASS = "text-[var(--scry-muted3)]";
const SUBPANEL_CLASS =
  "rounded-[12px] border border-[var(--scry-border3)] bg-[rgba(255,255,255,0.02)] p-3";

/** Index-based keys keep the selection stable across re-parses of the SVG. */
function indexKeys(): () => string {
  let next = 0;
  return () => String(next++);
}

type SettingsPosterOverlayTemplateEditorProps = {
  overview: PosterOverlayOverview;
  draft: PosterOverlayTemplateDraft;
  setDraft: React.Dispatch<React.SetStateAction<PosterOverlayTemplateDraft | null>>;
  busy: boolean;
  onSave: (event: React.FormEvent<HTMLFormElement>) => void;
  onValidate: () => void;
  validation: PosterOverlayTemplateValidation | null;
  preview: PosterOverlayPreviewState;
  sample: PosterOverlaySample;
  setSample: React.Dispatch<React.SetStateAction<PosterOverlaySample>>;
};

export function SettingsPosterOverlayTemplateEditor({
  overview,
  draft,
  setDraft,
  busy,
  onSave,
  onValidate,
  validation,
  preview,
  sample,
  setSample,
}: SettingsPosterOverlayTemplateEditorProps) {
  const t = useTranslate();
  const elements = React.useMemo(() => parseOverlay(draft.svg, indexKeys()), [draft.svg]);
  const [selected, setSelected] = React.useState(0);
  const [svgOpen, setSvgOpen] = React.useState(false);

  const selectedIndex =
    elements && elements.length > 0 ? Math.min(selected, elements.length - 1) : -1;
  const selectedElement = selectedIndex >= 0 ? (elements?.[selectedIndex] ?? null) : null;

  const writeElements = React.useCallback(
    (next: OverlayElement[]) => {
      setDraft((current) => (current ? { ...current, svg: serializeOverlay(next) } : current));
    },
    [setDraft],
  );

  const updateElement = React.useCallback(
    (index: number, patch: Partial<OverlayElement>, clamp = false) => {
      if (!elements) {
        return;
      }
      writeElements(
        elements.map((element, position) => {
          if (position !== index) {
            return element;
          }
          const next = { ...element, ...patch };
          return clamp ? clampElement(next) : next;
        }),
      );
    },
    [elements, writeElements],
  );

  const addElement = (kind: OverlayBadgeKind) => {
    const next = [...(elements ?? []), newElement(kind, String(elements?.length ?? 0))];
    writeElements(next);
    setSelected(next.length - 1);
  };

  const removeElement = (index: number) => {
    if (!elements) {
      return;
    }
    writeElements(elements.filter((_, position) => position !== index));
    setSelected(Math.max(0, index - 1));
  };

  const kindLabel = (kind: OverlayBadgeKind): string => {
    switch (kind) {
      case "resolution":
        return t("settings.posterOverlays.badgeKindResolution");
      case "hdr":
        return t("settings.posterOverlays.badgeKindHdr");
      case "audio":
        return t("settings.posterOverlays.badgeKindAudio");
      case "edition":
        return t("settings.posterOverlays.badgeKindEdition");
      default:
        return t("settings.posterOverlays.badgeKindCustom");
    }
  };

  return (
    <form
      id="settings-poster-overlays-template-form"
      className="space-y-4 border-t border-[var(--scry-border3)] p-4 sm:p-5"
      onSubmit={onSave}
    >
      <div className="space-y-1.5">
        <Label htmlFor="settings-poster-overlays-template-name">
          {t("settings.posterOverlays.templateName")}
        </Label>
        <Input
          id="settings-poster-overlays-template-name"
          value={draft.name}
          disabled={busy}
          onChange={(event) =>
            setDraft((current) => (current ? { ...current, name: event.target.value } : current))
          }
        />
      </div>

      <div className="grid gap-5 lg:grid-cols-[minmax(0,1fr)_minmax(260px,340px)]">
        <div className="min-w-0 space-y-4">
          {elements ? (
            <>
              <div className="space-y-2">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="font-medium text-[var(--scry-ink2)]">
                    {t("settings.posterOverlays.addBadge")}
                  </span>
                  {BADGE_KINDS.map((kind) => (
                    <Button
                      key={kind}
                      id={selectorId("settings-poster-overlays-add-badge", kind)}
                      type="button"
                      variant="secondary"
                      size="sm"
                      disabled={busy}
                      onClick={() => addElement(kind)}
                    >
                      <Plus className="size-3.5" />
                      {kindLabel(kind)}
                    </Button>
                  ))}
                </div>
                {elements.length === 0 ? (
                  <p className={MUTED_TEXT_CLASS}>{t("settings.posterOverlays.noBadges")}</p>
                ) : (
                  <ul
                    id="settings-poster-overlays-badges"
                    className="flex flex-wrap gap-2"
                    aria-label={t("settings.posterOverlays.badges")}
                  >
                    {elements.map((element, index) => (
                      <li key={element.key}>
                        <button
                          type="button"
                          id={selectorId("settings-poster-overlays-badge", String(index))}
                          aria-pressed={index === selectedIndex}
                          className={`rounded-[10px] border px-3 py-1.5 text-left text-xs transition-colors ${
                            index === selectedIndex
                              ? "border-[var(--scry-accent,#5b64ff)] bg-[rgba(91,100,255,0.16)] text-[var(--scry-ink2)]"
                              : "border-[var(--scry-border3)] text-[var(--scry-muted3)] hover:text-[var(--scry-ink2)]"
                          }`}
                          onClick={() => setSelected(index)}
                        >
                          <span className="font-semibold">{kindLabel(badgeKind(element))}</span>
                          <BadgeChipDetail element={element} />
                        </button>
                      </li>
                    ))}
                  </ul>
                )}
              </div>

              {selectedElement ? (
                <BadgeFields
                  element={selectedElement}
                  index={selectedIndex}
                  busy={busy}
                  title={kindLabel(badgeKind(selectedElement))}
                  sampleOptions={overview.sampleOptions}
                  onChange={(patch, clamp) => updateElement(selectedIndex, patch, clamp)}
                  onRemove={() => removeElement(selectedIndex)}
                />
              ) : null}

              <p className={`text-xs ${MUTED_TEXT_CLASS}`}>
                {t("settings.posterOverlays.coordinatesHelp", {
                  width: OVERLAY_CANVAS_WIDTH,
                  height: OVERLAY_CANVAS_HEIGHT,
                })}
              </p>
            </>
          ) : (
            <div
              id="settings-poster-overlays-unsupported"
              className={`${SUBPANEL_CLASS} space-y-2`}
            >
              <p>{t("settings.posterOverlays.unsupportedTemplate")}</p>
              <Button
                id="settings-poster-overlays-reset-builtin"
                type="button"
                variant="secondary"
                size="sm"
                disabled={busy}
                onClick={() =>
                  setDraft((current) =>
                    current ? { ...current, svg: overview.builtinTemplate } : current,
                  )
                }
              >
                {t("settings.posterOverlays.resetToBuiltin")}
              </Button>
            </div>
          )}

          <Collapsible open={svgOpen || !elements} onOpenChange={setSvgOpen}>
            {elements ? (
              <CollapsibleTrigger asChild>
                <button
                  id="settings-poster-overlays-svg-toggle"
                  type="button"
                  className={`flex items-center gap-1 text-xs ${MUTED_TEXT_CLASS} hover:text-[var(--scry-ink2)]`}
                >
                  <ChevronDown
                    className={`size-3.5 transition-transform ${svgOpen ? "" : "-rotate-90"}`}
                  />
                  {t("settings.posterOverlays.editAsSvg")}
                </button>
              </CollapsibleTrigger>
            ) : null}
            <CollapsibleContent className="mt-2 space-y-1.5">
              <Label htmlFor="settings-poster-overlays-template-svg">
                {t("settings.posterOverlays.templateSvg")}
              </Label>
              <Textarea
                id="settings-poster-overlays-template-svg"
                className="min-h-[240px] font-mono text-xs"
                spellCheck={false}
                value={draft.svg}
                disabled={busy}
                onChange={(event) =>
                  setDraft((current) =>
                    current ? { ...current, svg: event.target.value } : current,
                  )
                }
              />
              <p className={`text-xs ${MUTED_TEXT_CLASS}`}>
                <Layers className="mr-1 inline size-3.5 align-[-2px]" />
                {t("settings.posterOverlays.templateHelp", {
                  version: overview.templateSpecVersion,
                  fields: overview.templateFields.join(", "),
                })}
              </p>
            </CollapsibleContent>
          </Collapsible>
        </div>

        <PreviewPanel
          elements={elements}
          selectedIndex={selectedIndex}
          onSelect={setSelected}
          onMove={(index, x, y) => updateElement(index, { x, y }, true)}
          preview={preview}
          sample={sample}
          setSample={setSample}
          overview={overview}
          busy={busy}
        />
      </div>

      {validation ? (
        <p
          id="settings-poster-overlays-template-validation"
          className={validation.valid ? "text-emerald-500" : "text-destructive"}
        >
          {validation.valid ? t("settings.posterOverlays.templateValid") : validation.error}
        </p>
      ) : null}
      <div className="flex flex-wrap gap-2">
        <Button id="settings-poster-overlays-template-save" type="submit" disabled={busy}>
          {t("label.save")}
        </Button>
        <Button
          id="settings-poster-overlays-template-validate"
          type="button"
          variant="secondary"
          disabled={busy}
          onClick={onValidate}
        >
          {t("settings.posterOverlays.validate")}
        </Button>
        <Button
          id="settings-poster-overlays-template-cancel"
          type="button"
          variant="ghost"
          disabled={busy}
          onClick={() => setDraft(null)}
        >
          {t("label.cancel")}
        </Button>
      </div>
    </form>
  );
}

/** What tells two badges of the same kind apart: a custom badge's text, or a narrowed condition such as `hdr=dv`. */
function BadgeChipDetail({ element }: { element: OverlayElement }) {
  const detail =
    badgeKind(element) === "custom"
      ? element.text
      : element.showWhen.includes("=")
        ? element.showWhen
        : "";
  return detail ? <span className="ml-1.5 opacity-80">{detail}</span> : null;
}

type BadgeFieldsProps = {
  element: OverlayElement;
  index: number;
  busy: boolean;
  title: string;
  sampleOptions: PosterOverlaySampleOptions;
  onChange: (patch: Partial<OverlayElement>, clamp?: boolean) => void;
  onRemove: () => void;
};

function BadgeFields({
  element,
  index,
  busy,
  title,
  sampleOptions,
  onChange,
  onRemove,
}: BadgeFieldsProps) {
  const t = useTranslate();
  const id = (field: string) =>
    selectorId("settings-poster-overlays-badge-field", `${index}-${field}`);

  return (
    <div id="settings-poster-overlays-badge-editor" className={`${SUBPANEL_CLASS} space-y-3`}>
      <div className="flex items-center justify-between gap-2">
        <h4 className="font-semibold text-[var(--scry-ink2)]">{title}</h4>
        <IconButton
          id="settings-poster-overlays-badge-remove"
          label={t("settings.posterOverlays.removeBadge")}
          disabled={busy}
          onClick={onRemove}
        >
          <Trash2 className="size-4" />
        </IconButton>
      </div>

      <div className="space-y-1.5">
        <Label htmlFor={id("text")}>{t("settings.posterOverlays.badgeText")}</Label>
        <Input
          id={id("text")}
          value={element.text}
          disabled={busy}
          onChange={(event) => onChange({ text: event.target.value })}
        />
        <p className={`text-xs ${MUTED_TEXT_CLASS}`}>
          {t("settings.posterOverlays.badgeTextHelp", { example: "{{audio_label}}" })}
        </p>
      </div>

      <BadgeConditionFields
        element={element}
        sampleOptions={sampleOptions}
        busy={busy}
        id={id}
        onChange={onChange}
      />

      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <NumberField
          id={id("x")}
          label={t("settings.posterOverlays.positionX")}
          value={element.x}
          busy={busy}
          onChange={(x, done) => onChange({ x }, done)}
        />
        <NumberField
          id={id("y")}
          label={t("settings.posterOverlays.positionY")}
          value={element.y}
          busy={busy}
          onChange={(y, done) => onChange({ y }, done)}
        />
        <NumberField
          id={id("width")}
          label={t("settings.posterOverlays.width")}
          value={element.width}
          busy={busy}
          onChange={(width, done) => onChange({ width }, done)}
        />
        <NumberField
          id={id("height")}
          label={t("settings.posterOverlays.height")}
          value={element.height}
          busy={busy}
          onChange={(height, done) => onChange({ height }, done)}
        />
        <NumberField
          id={id("font-size")}
          label={t("settings.posterOverlays.fontSize")}
          value={element.fontSize}
          busy={busy}
          onChange={(fontSize, done) => onChange({ fontSize }, done)}
        />
        <NumberField
          id={id("letter-spacing")}
          label={t("settings.posterOverlays.letterSpacing")}
          value={element.letterSpacing}
          busy={busy}
          onChange={(letterSpacing, done) => onChange({ letterSpacing }, done)}
        />
        <NumberField
          id={id("radius")}
          label={t("settings.posterOverlays.cornerRadius")}
          value={element.radius}
          busy={busy}
          onChange={(radius, done) => onChange({ radius }, done)}
        />
        <NumberField
          id={id("opacity")}
          label={t("settings.posterOverlays.backgroundOpacity")}
          value={Math.round(element.backgroundOpacity * 100)}
          busy={busy}
          onChange={(percent, done) => onChange({ backgroundOpacity: percent / 100 }, done)}
        />
      </div>

      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
        <div className="space-y-1.5">
          <Label htmlFor={id("text-color")}>{t("settings.posterOverlays.textColor")}</Label>
          <Input
            id={id("text-color")}
            type="color"
            className="h-9 p-1"
            value={element.textColor}
            disabled={busy}
            onChange={(event) => onChange({ textColor: event.target.value })}
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor={id("background")}>{t("settings.posterOverlays.backgroundColor")}</Label>
          <Input
            id={id("background")}
            type="color"
            className="h-9 p-1"
            value={element.background}
            disabled={busy}
            onChange={(event) => onChange({ background: event.target.value })}
          />
        </div>
        <div className="col-span-2 space-y-1.5 sm:col-span-1">
          <Label htmlFor={id("align")}>{t("settings.posterOverlays.textAlign")}</Label>
          <Select
            value={element.align}
            disabled={busy}
            onValueChange={(align) => onChange({ align: align as OverlayTextAlign })}
          >
            <SelectTrigger id={id("align")} className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="start">{t("settings.posterOverlays.alignLeft")}</SelectItem>
              <SelectItem value="middle">{t("settings.posterOverlays.alignCenter")}</SelectItem>
              <SelectItem value="end">{t("settings.posterOverlays.alignRight")}</SelectItem>
            </SelectContent>
          </Select>
        </div>
      </div>
    </div>
  );
}

/** Radix Select forbids an empty value, so "no field" needs a token. */
const NO_FIELD_VALUE = "__always__";

type ConditionOption = { value: string; label: string };

/** Values each field can take, labelled as the badge prints them. */
function conditionOptions(
  field: ConditionField,
  sampleOptions: PosterOverlaySampleOptions,
): ConditionOption[] | null {
  const labelled = (options: { token: string; label: string }[]) =>
    options.map((option) => ({ value: option.token, label: `${option.label} (${option.token})` }));
  switch (field) {
    case "resolution":
      return labelled(sampleOptions.resolutions);
    case "hdr":
      return labelled(sampleOptions.hdr);
    case "audio_codec":
      return labelled(sampleOptions.audio);
    case "audio_channels":
      return sampleOptions.audioChannels.map((layout) => ({ value: layout, label: layout }));
    default:
      return null;
  }
}

type BadgeConditionFieldsProps = {
  element: OverlayElement;
  sampleOptions: PosterOverlaySampleOptions;
  busy: boolean;
  id: (field: string) => string;
  onChange: (patch: Partial<OverlayElement>) => void;
};

/**
 * Show when / Hide when as dropdowns: pick the field, then the values to show
 * for ("All" means any value) and the values to hide for. Hide wins. Conditions
 * the dropdowns cannot express stay editable as text.
 */
function BadgeConditionFields({
  element,
  sampleOptions,
  busy,
  id,
  onChange,
}: BadgeConditionFieldsProps) {
  const t = useTranslate();
  const condition = parseBadgeCondition(element.showWhen, element.hideWhen);

  if (!condition) {
    return (
      <div className="space-y-1.5">
        <div className="grid gap-3 sm:grid-cols-2">
          <div className="space-y-1.5">
            <Label htmlFor={id("show")}>{t("settings.posterOverlays.showWhen")}</Label>
            <Input
              id={id("show")}
              value={element.showWhen}
              disabled={busy}
              spellCheck={false}
              className="font-mono text-xs"
              onChange={(event) => onChange({ showWhen: event.target.value })}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor={id("hide")}>{t("settings.posterOverlays.hideWhen")}</Label>
            <Input
              id={id("hide")}
              value={element.hideWhen}
              disabled={busy}
              spellCheck={false}
              className="font-mono text-xs"
              onChange={(event) => onChange({ hideWhen: event.target.value })}
            />
          </div>
        </div>
        <p className={`text-xs ${MUTED_TEXT_CLASS}`}>
          {t("settings.posterOverlays.conditionTextOnly")}
        </p>
      </div>
    );
  }

  const write = (next: BadgeCondition) => onChange(formatBadgeCondition(next));
  const fieldLabel = (field: ConditionField): string => {
    switch (field) {
      case "resolution":
        return t("settings.posterOverlays.badgeKindResolution");
      case "hdr":
        return t("settings.posterOverlays.badgeKindHdr");
      case "audio_codec":
        return t("settings.posterOverlays.badgeKindAudio");
      case "audio_channels":
        return t("settings.posterOverlays.sampleChannels");
      default:
        return t("settings.posterOverlays.badgeKindEdition");
    }
  };
  const options = condition.field ? conditionOptions(condition.field, sampleOptions) : null;
  const summarise = (values: string[], empty: string) =>
    values.length === 0
      ? empty
      : values
          .map((value) => options?.find((option) => option.value === value)?.label ?? value)
          .join(", ");

  return (
    <div className="space-y-3">
      <div className="space-y-1.5">
        <Label htmlFor={id("condition-field")}>{t("settings.posterOverlays.conditionField")}</Label>
        <Select
          value={condition.field ?? NO_FIELD_VALUE}
          disabled={busy}
          onValueChange={(value) =>
            write({
              field: value === NO_FIELD_VALUE ? null : (value as ConditionField),
              show: "all",
              hide: [],
            })
          }
        >
          <SelectTrigger id={id("condition-field")} className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={NO_FIELD_VALUE}>
              {t("settings.posterOverlays.conditionFieldNone")}
            </SelectItem>
            {CONDITION_FIELDS.map((field) => (
              <SelectItem key={field} value={field}>
                {fieldLabel(field)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      {condition.field && options ? (
        <div className="grid gap-3 sm:grid-cols-2">
          <div className="space-y-1.5">
            <Label htmlFor={id("show")}>{t("settings.posterOverlays.showWhen")}</Label>
            <MultiSelectDropdown
              id={id("show")}
              options={options}
              selectedValues={condition.show === "all" ? [] : condition.show}
              onSelectedValuesChange={(values) =>
                write({ ...condition, show: values.length > 0 ? values : "all" })
              }
              allOption={{
                label: t("settings.posterOverlays.conditionAll"),
                selected: condition.show === "all",
                onSelect: () => write({ ...condition, show: "all" }),
                id: `${id("show")}-all`,
              }}
              triggerLabel={
                condition.show === "all"
                  ? t("settings.posterOverlays.conditionAll")
                  : summarise(condition.show, t("settings.posterOverlays.conditionAll"))
              }
              disabled={busy}
              optionIdPrefix={`${id("show")}-option`}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor={id("hide")}>{t("settings.posterOverlays.hideWhen")}</Label>
            <MultiSelectDropdown
              id={id("hide")}
              options={options}
              selectedValues={condition.hide}
              onSelectedValuesChange={(values) => write({ ...condition, hide: values })}
              triggerLabel={summarise(condition.hide, t("settings.posterOverlays.conditionNever"))}
              disabled={busy}
              optionIdPrefix={`${id("hide")}-option`}
            />
          </div>
        </div>
      ) : null}

      {condition.field === "edition" ? (
        <div className="grid gap-3 sm:grid-cols-2">
          <EditionValuesField
            id={id("show")}
            label={t("settings.posterOverlays.showWhen")}
            placeholder={t("settings.posterOverlays.conditionAll")}
            values={condition.show === "all" ? [] : condition.show}
            busy={busy}
            onChange={(values) => write({ ...condition, show: values.length > 0 ? values : "all" })}
          />
          <EditionValuesField
            id={id("hide")}
            label={t("settings.posterOverlays.hideWhen")}
            placeholder={t("settings.posterOverlays.conditionNever")}
            values={condition.hide}
            busy={busy}
            onChange={(values) => write({ ...condition, hide: values })}
          />
        </div>
      ) : null}

      <p className={`text-xs ${MUTED_TEXT_CLASS}`}>
        {condition.field === "edition"
          ? t("settings.posterOverlays.editionValuesHelp")
          : t("settings.posterOverlays.conditionHelp")}
      </p>
    </div>
  );
}

type EditionValuesFieldProps = {
  id: string;
  label: string;
  placeholder: string;
  values: string[];
  busy: boolean;
  onChange: (values: string[]) => void;
};

/**
 * Edition names, comma separated. Converted to condition values when the box
 * loses focus, so typing "Director's" is not rewritten under the cursor.
 */
function EditionValuesField({
  id,
  label,
  placeholder,
  values,
  busy,
  onChange,
}: EditionValuesFieldProps) {
  const [text, setText] = React.useState(values.join(", "));
  const [focused, setFocused] = React.useState(false);
  const joined = values.join(", ");
  React.useEffect(() => {
    if (!focused) {
      setText(joined);
    }
  }, [focused, joined]);

  return (
    <div className="space-y-1.5">
      <Label htmlFor={id}>{label}</Label>
      <Input
        id={id}
        value={text}
        placeholder={placeholder}
        disabled={busy}
        onFocus={() => setFocused(true)}
        onChange={(event) => setText(event.target.value)}
        onBlur={() => {
          setFocused(false);
          onChange([
            ...new Set(
              text
                .split(",")
                .map(editionToken)
                .filter((value) => value.length > 0),
            ),
          ]);
        }}
      />
    </div>
  );
}

type NumberFieldProps = {
  id: string;
  label: string;
  value: number;
  busy: boolean;
  /** `done` is true on blur, when the value is also clamped to the poster. */
  onChange: (value: number, done: boolean) => void;
};

/**
 * Number input that keeps what the user is typing (an empty box, a lone `-`)
 * and only reports values that parse. Clamping waits for blur so typing
 * "300" is not pushed around after the "3".
 */
function NumberField({ id, label, value, busy, onChange }: NumberFieldProps) {
  const [text, setText] = React.useState(String(value));
  const [focused, setFocused] = React.useState(false);
  React.useEffect(() => {
    if (!focused) {
      setText(String(value));
    }
  }, [focused, value]);

  return (
    <div className="space-y-1.5">
      <Label htmlFor={id}>{label}</Label>
      <Input
        id={id}
        type="number"
        inputMode="decimal"
        value={text}
        disabled={busy}
        onFocus={() => setFocused(true)}
        onChange={(event) => {
          setText(event.target.value);
          const parsed = Number.parseFloat(event.target.value);
          if (Number.isFinite(parsed)) {
            onChange(parsed, false);
          }
        }}
        onBlur={() => {
          setFocused(false);
          const parsed = Number.parseFloat(text);
          onChange(Number.isFinite(parsed) ? parsed : value, true);
        }}
      />
    </div>
  );
}

type PreviewPanelProps = {
  elements: OverlayElement[] | null;
  selectedIndex: number;
  onSelect: (index: number) => void;
  onMove: (index: number, x: number, y: number) => void;
  preview: PosterOverlayPreviewState;
  sample: PosterOverlaySample;
  setSample: React.Dispatch<React.SetStateAction<PosterOverlaySample>>;
  overview: PosterOverlayOverview;
  busy: boolean;
};

type DragState = {
  index: number;
  pointerId: number;
  startX: number;
  startY: number;
  originX: number;
  originY: number;
};

function PreviewPanel({
  elements,
  selectedIndex,
  onSelect,
  onMove,
  preview,
  sample,
  setSample,
  overview,
  busy,
}: PreviewPanelProps) {
  const t = useTranslate();
  const surface = React.useRef<SVGSVGElement>(null);
  const drag = React.useRef<DragState | null>(null);
  const options = overview.sampleOptions;

  /** Pointer position in template units. */
  const toCanvas = (event: React.PointerEvent): { x: number; y: number } | null => {
    const bounds = surface.current?.getBoundingClientRect();
    if (!bounds || bounds.width === 0 || bounds.height === 0) {
      return null;
    }
    return {
      x: ((event.clientX - bounds.left) / bounds.width) * OVERLAY_CANVAS_WIDTH,
      y: ((event.clientY - bounds.top) / bounds.height) * OVERLAY_CANVAS_HEIGHT,
    };
  };

  const setSampleField = (field: keyof PosterOverlaySample) => (value: string) =>
    setSample((current) => ({ ...current, [field]: value === NONE_VALUE ? "" : value }));

  return (
    <div className="space-y-3 lg:sticky lg:top-4 lg:self-start">
      <div className="flex items-center justify-between gap-2">
        <span className="font-medium text-[var(--scry-ink2)]">
          {t("settings.posterOverlays.preview")}
        </span>
        {preview.loading ? (
          <Loader2
            className="size-4 animate-spin text-[var(--scry-muted3)]"
            aria-label={t("settings.posterOverlays.previewRendering")}
          />
        ) : null}
      </div>
      <div className="relative mx-auto aspect-[2/3] w-full max-w-[340px] overflow-hidden rounded-[10px] border border-[var(--scry-border3)] bg-[#141823]">
        {preview.image ? (
          <img
            id="settings-poster-overlays-preview-image"
            src={preview.image}
            alt={t("settings.posterOverlays.preview")}
            className="absolute inset-0 size-full select-none object-cover"
            draggable={false}
          />
        ) : null}
        {elements ? (
          <svg
            ref={surface}
            viewBox={`0 0 ${OVERLAY_CANVAS_WIDTH} ${OVERLAY_CANVAS_HEIGHT}`}
            className="absolute inset-0 size-full touch-none"
            aria-hidden="true"
          >
            {elements.map((element, index) => (
              <rect
                key={element.key}
                x={element.x}
                y={element.y}
                width={element.width}
                height={element.height}
                fill="transparent"
                stroke={index === selectedIndex ? "#38bdf8" : "transparent"}
                strokeWidth={6}
                strokeDasharray="18 10"
                className={busy ? "" : "cursor-move"}
                onPointerDown={(event) => {
                  if (busy) {
                    return;
                  }
                  const point = toCanvas(event);
                  if (!point) {
                    return;
                  }
                  event.currentTarget.setPointerCapture(event.pointerId);
                  onSelect(index);
                  drag.current = {
                    index,
                    pointerId: event.pointerId,
                    startX: point.x,
                    startY: point.y,
                    originX: element.x,
                    originY: element.y,
                  };
                }}
                onPointerMove={(event) => {
                  const state = drag.current;
                  const point = toCanvas(event);
                  if (!state || state.pointerId !== event.pointerId || !point) {
                    return;
                  }
                  onMove(
                    state.index,
                    Math.round(state.originX + point.x - state.startX),
                    Math.round(state.originY + point.y - state.startY),
                  );
                }}
                onPointerUp={() => {
                  drag.current = null;
                }}
                onPointerCancel={() => {
                  drag.current = null;
                }}
              />
            ))}
          </svg>
        ) : null}
      </div>
      {preview.error ? (
        <p id="settings-poster-overlays-preview-error" className="text-xs text-destructive">
          {preview.error}
        </p>
      ) : (
        <p className={`text-xs ${MUTED_TEXT_CLASS}`}>
          {preview.libraryPoster
            ? t("settings.posterOverlays.previewLibraryPoster")
            : t("settings.posterOverlays.previewPlaceholder")}
        </p>
      )}

      <div className={`${SUBPANEL_CLASS} space-y-2.5`}>
        <span className="text-xs font-semibold uppercase tracking-wide text-[var(--scry-muted3)]">
          {t("settings.posterOverlays.sampleTitle")}
        </span>
        <div className="grid grid-cols-2 gap-2.5">
          <SampleSelect
            id="settings-poster-overlays-sample-resolution"
            label={t("settings.posterOverlays.badgeKindResolution")}
            value={sample.resolution}
            options={options.resolutions.map((option) => ({
              value: option.token,
              label: option.label,
            }))}
            onChange={setSampleField("resolution")}
          />
          <SampleSelect
            id="settings-poster-overlays-sample-hdr"
            label={t("settings.posterOverlays.badgeKindHdr")}
            value={sample.hdr}
            options={options.hdr.map((option) => ({ value: option.token, label: option.label }))}
            onChange={setSampleField("hdr")}
          />
          <SampleSelect
            id="settings-poster-overlays-sample-audio"
            label={t("settings.posterOverlays.badgeKindAudio")}
            value={sample.audio}
            options={options.audio.map((option) => ({ value: option.token, label: option.label }))}
            onChange={setSampleField("audio")}
          />
          <SampleSelect
            id="settings-poster-overlays-sample-channels"
            label={t("settings.posterOverlays.sampleChannels")}
            value={sample.audioChannels}
            options={options.audioChannels.map((layout) => ({ value: layout, label: layout }))}
            onChange={setSampleField("audioChannels")}
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="settings-poster-overlays-sample-edition" className="text-xs">
            {t("settings.posterOverlays.badgeKindEdition")}
          </Label>
          <Input
            id="settings-poster-overlays-sample-edition"
            value={sample.edition}
            maxLength={80}
            placeholder={t("settings.posterOverlays.sampleNone")}
            onChange={(event) => setSampleField("edition")(event.target.value)}
          />
        </div>
      </div>
    </div>
  );
}

type SampleSelectProps = {
  id: string;
  label: string;
  value: string;
  options: { value: string; label: string }[];
  onChange: (value: string) => void;
};

function SampleSelect({ id, label, value, options, onChange }: SampleSelectProps) {
  const t = useTranslate();
  return (
    <div className="space-y-1.5">
      <Label htmlFor={id} className="text-xs">
        {label}
      </Label>
      <Select value={value || NONE_VALUE} onValueChange={onChange}>
        <SelectTrigger id={id} className="h-8 w-full text-xs">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value={NONE_VALUE}>{t("settings.posterOverlays.sampleNone")}</SelectItem>
          {options.map((option) => (
            <SelectItem key={option.value} value={option.value}>
              {option.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  );
}
