/**
 * Visual editing model for poster overlay templates.
 *
 * The editor works on a list of badges, each a background box with one line
 * of text, and writes them out as an ordinary template SVG. Parsing only
 * accepts SVG in exactly that shape (which the built-in template uses), so a
 * hand-written template with other elements is never silently rewritten: it
 * stays editable as raw SVG instead.
 */

/** Template coordinate space; it scales to each poster. 2:3 like a poster. */
export const OVERLAY_CANVAS_WIDTH = 1000;
export const OVERLAY_CANVAS_HEIGHT = 1500;

export const MIN_FONT_SIZE = 6;
export const MAX_FONT_SIZE = 400;

export type OverlayTextAlign = "start" | "middle" | "end";

export type OverlayBadgeKind =
  "resolution" | "hdr" | "codec" | "source" | "audio" | "edition" | "status" | "ratings" | "custom";

/**
 * Rating sources a badge can show, in display order. `field` holds the
 * title's score as the web UI writes it; `logo` names the bundled logo the
 * server draws for `<use href="#scryer-logo-NAME"/>`.
 */
export const RATING_SOURCES = [
  { token: "imdb", label: "IMDb", logo: "imdb" },
  { token: "rottentomatoes", label: "Rotten Tomatoes", logo: "rottentomatoes" },
  { token: "popcornmeter", label: "Popcornmeter", logo: "popcornmeter" },
  { token: "metacritic", label: "Metacritic", logo: "metacritic" },
  { token: "metacritic_user", label: "Metacritic User", logo: "metacritic" },
  { token: "letterboxd", label: "Letterboxd", logo: "letterboxd" },
  { token: "tmdb", label: "TMDB", logo: "tmdb" },
  { token: "trakt", label: "Trakt", logo: "trakt" },
  { token: "mdblist", label: "MDBList", logo: "mdblist" },
] as const;

export type RatingSourceToken = (typeof RATING_SOURCES)[number]["token"];

export function ratingField(token: RatingSourceToken): string {
  return `rating_${token}`;
}

export type OverlayStackDirection = "down" | "up" | "right" | "left";

/** `above`: logo over the score, as Kometa draws them. `beside`: logo left. */
export type OverlayRatingLayout = "above" | "beside";

/**
 * A ratings badge: one tile per source, each a box with the source's logo and
 * score. Tiles for sources a title has no score from are left out and the
 * rest close up, so the first tile sits at the badge's position.
 */
export type OverlayRatings = {
  sources: RatingSourceToken[];
  direction: OverlayStackDirection;
  /** Space between tiles, in template units. */
  gap: number;
  layout: OverlayRatingLayout;
};

export type OverlayElement = {
  /** Client-side identity for React keys and selection; never serialised. */
  key: string;
  /** `data-scryer-if`; empty means always shown. */
  showWhen: string;
  /** `data-scryer-unless`; empty means never hidden. */
  hideWhen: string;
  /** Text with `{{field}}` placeholders. */
  text: string;
  x: number;
  y: number;
  width: number;
  height: number;
  fontSize: number;
  radius: number;
  /** `#rrggbb`. */
  textColor: string;
  /** `#rrggbb`. */
  background: string;
  /** 0 (no box) to 1 (solid). */
  backgroundOpacity: number;
  align: OverlayTextAlign;
  letterSpacing: number;
  /** Set for a ratings badge; its box is then one tile. */
  ratings: OverlayRatings | null;
};

type BadgePreset = { showWhen: string; hideWhen: string; text: string };

export const BADGE_PRESETS: Record<Exclude<OverlayBadgeKind, "custom" | "ratings">, BadgePreset> = {
  resolution: {
    showWhen: "resolution",
    hideWhen: "",
    text: "{{resolution_label}}",
  },
  hdr: { showWhen: "hdr", hideWhen: "hdr=sdr", text: "{{hdr_label}}" },
  codec: {
    showWhen: "video_codec",
    hideWhen: "",
    text: "{{video_codec_label}}",
  },
  source: { showWhen: "source", hideWhen: "", text: "{{source_label}}" },
  audio: {
    showWhen: "audio_codec",
    hideWhen: "audio_codec=other",
    text: "{{audio_label}} {{audio_channels}}",
  },
  edition: { showWhen: "edition", hideWhen: "", text: "{{edition_label}}" },
  status: {
    showWhen: "series_status",
    hideWhen: "",
    text: "{{series_status_label}}",
  },
};

export const BADGE_KINDS: OverlayBadgeKind[] = [
  "resolution",
  "hdr",
  "codec",
  "source",
  "audio",
  "edition",
  "status",
  "ratings",
  "custom",
];

/** Where a newly added badge of each kind lands. */
const KIND_DEFAULT_BOX: Record<
  OverlayBadgeKind,
  Pick<OverlayElement, "x" | "y" | "width" | "height" | "fontSize">
> = {
  resolution: { x: 40, y: 40, width: 270, height: 110, fontSize: 62 },
  hdr: { x: 610, y: 40, width: 350, height: 110, fontSize: 48 },
  codec: { x: 40, y: 170, width: 270, height: 90, fontSize: 48 },
  source: { x: 690, y: 170, width: 270, height: 90, fontSize: 44 },
  audio: { x: 460, y: 1350, width: 500, height: 110, fontSize: 50 },
  edition: { x: 40, y: 180, width: 420, height: 72, fontSize: 32 },
  status: { x: 40, y: 1350, width: 380, height: 90, fontSize: 40 },
  ratings: { x: 40, y: 300, width: 200, height: 200, fontSize: 54 },
  custom: { x: 350, y: 700, width: 300, height: 100, fontSize: 48 },
};

/** The field a badge's condition tests, used to label it in the editor. */
export function badgeKind(
  element: Pick<OverlayElement, "showWhen"> & {
    ratings?: OverlayRatings | null;
  },
): OverlayBadgeKind {
  if (element.ratings) {
    return "ratings";
  }
  const field = element.showWhen.split(/[=|]/, 1)[0]?.trim() ?? "";
  switch (field) {
    case "resolution":
      return "resolution";
    case "hdr":
      return "hdr";
    case "audio_codec":
      return "audio";
    case "edition":
      return "edition";
    case "series_status":
      return "status";
    case "video_codec":
      return "codec";
    case "source":
      return "source";
    default:
      return "custom";
  }
}

export function newElement(kind: OverlayBadgeKind, key: string): OverlayElement {
  const preset =
    kind === "custom"
      ? { showWhen: "", hideWhen: "", text: "TEXT" }
      : kind === "ratings"
        ? { showWhen: "", hideWhen: "", text: "" }
        : BADGE_PRESETS[kind];
  return {
    key,
    ...preset,
    ...KIND_DEFAULT_BOX[kind],
    radius: 22,
    textColor: kind === "hdr" ? "#f5c542" : "#ffffff",
    background: "#0b0d12",
    backgroundOpacity: 0.82,
    align: "middle",
    letterSpacing: 0,
    ratings:
      kind === "ratings"
        ? {
            sources: ["rottentomatoes", "imdb", "tmdb"],
            direction: "down",
            gap: 20,
            layout: "above",
          }
        : null,
  };
}

/** Distance from one tile of a ratings badge to the next. */
function stackStep(element: OverlayElement, ratings: OverlayRatings): number {
  const along =
    ratings.direction === "down" || ratings.direction === "up" ? element.height : element.width;
  return along + ratings.gap;
}

/**
 * The area a badge covers: its box, or for a ratings badge every tile it
 * draws when the title has a score from every chosen source.
 */
export function elementBounds(element: OverlayElement): {
  x: number;
  y: number;
  width: number;
  height: number;
} {
  const ratings = element.ratings;
  if (!ratings || ratings.sources.length < 2) {
    return {
      x: element.x,
      y: element.y,
      width: element.width,
      height: element.height,
    };
  }
  const span = stackStep(element, ratings) * (ratings.sources.length - 1);
  switch (ratings.direction) {
    case "down":
      return {
        x: element.x,
        y: element.y,
        width: element.width,
        height: element.height + span,
      };
    case "up":
      return {
        x: element.x,
        y: element.y - span,
        width: element.width,
        height: element.height + span,
      };
    case "right":
      return {
        x: element.x,
        y: element.y,
        width: element.width + span,
        height: element.height,
      };
    case "left":
      return {
        x: element.x - span,
        y: element.y,
        width: element.width + span,
        height: element.height,
      };
  }
}

function clamp(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) {
    return min;
  }
  return Math.min(max, Math.max(min, value));
}

/** Keep a badge inside the canvas with usable dimensions. */
export function clampElement(element: OverlayElement): OverlayElement {
  const width = clamp(element.width, 1, OVERLAY_CANVAS_WIDTH);
  const height = clamp(element.height, 1, OVERLAY_CANVAS_HEIGHT);
  return {
    ...element,
    width,
    height,
    x: clamp(element.x, 0, OVERLAY_CANVAS_WIDTH - width),
    y: clamp(element.y, 0, OVERLAY_CANVAS_HEIGHT - height),
    fontSize: clamp(element.fontSize, MIN_FONT_SIZE, MAX_FONT_SIZE),
    radius: clamp(element.radius, 0, Math.min(width, height) / 2),
    backgroundOpacity: clamp(element.backgroundOpacity, 0, 1),
    letterSpacing: clamp(element.letterSpacing, -20, 100),
    ratings: element.ratings
      ? {
          ...element.ratings,
          gap: clamp(element.ratings.gap, 0, OVERLAY_CANVAS_HEIGHT),
        }
      : null,
  };
}

/** Inter's cap height is about 0.73em; half of it centres capitals in the box. */
const CAP_CENTER_RATIO = 0.36;

function textAnchorX(element: OverlayElement): number {
  const padding = Math.max(8, element.fontSize * 0.4);
  switch (element.align) {
    case "start":
      return element.x + padding;
    case "end":
      return element.x + element.width - padding;
    default:
      return element.x + element.width / 2;
  }
}

function formatNumber(value: number): string {
  return String(Math.round(value * 100) / 100);
}

function escapeText(value: string): string {
  return value
    .replace(/&(?!(?:amp|lt|gt|quot|apos|#\d+|#x[0-9a-fA-F]+);)/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

function escapeAttribute(value: string): string {
  return escapeText(value).replace(/"/g, "&quot;");
}

function unescapeXml(value: string): string {
  return value
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&amp;/g, "&");
}

/** Logo box and score position inside one ratings tile. */
function ratingTileLayout(element: OverlayElement, layout: OverlayRatingLayout) {
  const padding = Math.round(Math.min(element.width, element.height) * 0.08);
  if (layout === "above") {
    const logoHeight = Math.round((element.height - padding * 2) * 0.55);
    const textTop = element.y + padding + logoHeight;
    return {
      logo: {
        x: element.x + padding,
        y: element.y + padding,
        width: element.width - padding * 2,
        height: logoHeight,
      },
      textX: element.x + element.width / 2,
      baseline:
        (textTop + element.y + element.height - padding) / 2 + element.fontSize * CAP_CENTER_RATIO,
    };
  }
  const logoWidth = Math.round((element.width - padding * 2) * 0.45);
  const textLeft = element.x + padding + logoWidth;
  return {
    logo: {
      x: element.x + padding,
      y: element.y + padding,
      width: logoWidth,
      height: element.height - padding * 2,
    },
    textX: (textLeft + element.x + element.width - padding) / 2,
    baseline: element.y + element.height / 2 + element.fontSize * CAP_CENTER_RATIO,
  };
}

function serializeRatings(element: OverlayElement, ratings: OverlayRatings): string {
  const tile = ratingTileLayout(element, ratings.layout);
  const rect = rectAttributes(element);
  const text = [
    `x="${formatNumber(tile.textX)}"`,
    `y="${formatNumber(tile.baseline)}"`,
    `font-family="Inter"`,
    `font-weight="700"`,
    `font-size="${formatNumber(element.fontSize)}"`,
    `fill="${element.textColor}"`,
    `text-anchor="middle"`,
    element.letterSpacing !== 0 ? `letter-spacing="${formatNumber(element.letterSpacing)}"` : "",
  ]
    .filter(Boolean)
    .join(" ");
  const logoBox = [
    `x="${formatNumber(tile.logo.x)}"`,
    `y="${formatNumber(tile.logo.y)}"`,
    `width="${formatNumber(tile.logo.width)}"`,
    `height="${formatNumber(tile.logo.height)}"`,
  ].join(" ");
  const tiles = ratings.sources.map((token) => {
    const source = RATING_SOURCES.find((candidate) => candidate.token === token)!;
    const field = ratingField(token);
    return [
      `    <g data-scryer-if="${field}">`,
      `      <rect ${rect}/>`,
      `      <use href="#scryer-logo-${source.logo}" ${logoBox}/>`,
      `      <text ${text}>{{${field}}}</text>`,
      `    </g>`,
    ].join("\n");
  });
  return [
    `  <g data-scryer-stack="${ratings.direction}" data-scryer-step="${formatNumber(stackStep(element, ratings))}">`,
    ...tiles,
    `  </g>`,
  ].join("\n");
}

function rectAttributes(element: OverlayElement): string {
  return [
    `x="${formatNumber(element.x)}"`,
    `y="${formatNumber(element.y)}"`,
    `width="${formatNumber(element.width)}"`,
    `height="${formatNumber(element.height)}"`,
    element.radius > 0 ? `rx="${formatNumber(element.radius)}"` : "",
    `fill="${element.background}"`,
    `fill-opacity="${formatNumber(element.backgroundOpacity)}"`,
  ]
    .filter(Boolean)
    .join(" ");
}

function serializeElement(element: OverlayElement): string {
  if (element.ratings) {
    return serializeRatings(element, element.ratings);
  }
  const groupAttributes = [
    element.showWhen.trim() ? ` data-scryer-if="${escapeAttribute(element.showWhen.trim())}"` : "",
    element.hideWhen.trim()
      ? ` data-scryer-unless="${escapeAttribute(element.hideWhen.trim())}"`
      : "",
  ].join("");
  const rect = rectAttributes(element);
  const baseline = element.y + element.height / 2 + element.fontSize * CAP_CENTER_RATIO;
  const text = [
    `x="${formatNumber(textAnchorX(element))}"`,
    `y="${formatNumber(baseline)}"`,
    `font-family="Inter"`,
    `font-weight="700"`,
    `font-size="${formatNumber(element.fontSize)}"`,
    `fill="${element.textColor}"`,
    `text-anchor="${element.align}"`,
    element.letterSpacing !== 0 ? `letter-spacing="${formatNumber(element.letterSpacing)}"` : "",
  ]
    .filter(Boolean)
    .join(" ");
  return [
    `  <g${groupAttributes}>`,
    `    <rect ${rect}/>`,
    `    <text ${text}>${escapeText(element.text)}</text>`,
    `  </g>`,
  ].join("\n");
}

export function serializeOverlay(elements: OverlayElement[]): string {
  return [
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${OVERLAY_CANVAS_WIDTH} ${OVERLAY_CANVAS_HEIGHT}" data-scryer-version="1">`,
    ...elements.map(serializeElement),
    `</svg>`,
    "",
  ].join("\n");
}

const ATTRIBUTE_PATTERN = /([\w:-]+)\s*=\s*"([^"]*)"/g;

/** Attributes of a start tag, or null if anything in it is not `name="value"`. */
function parseAttributes(source: string): Map<string, string> | null {
  const attributes = new Map<string, string>();
  const rest = source.replace(ATTRIBUTE_PATTERN, (_, name: string, value: string) => {
    attributes.set(name, unescapeXml(value));
    return "";
  });
  return rest.trim() === "" ? attributes : null;
}

function onlyKnown(attributes: Map<string, string>, allowed: readonly string[]): boolean {
  return [...attributes.keys()].every((name) => allowed.includes(name));
}

function parseNumber(value: string | undefined, fallback: number): number | null {
  if (value === undefined) {
    return fallback;
  }
  const parsed = Number(value.trim());
  return Number.isFinite(parsed) ? parsed : null;
}

/** `#rgb` or `#rrggbb` as lowercase `#rrggbb`; anything else is unsupported. */
function parseHexColor(value: string | undefined, fallback: string): string | null {
  if (value === undefined) {
    return fallback;
  }
  const trimmed = value.trim().toLowerCase();
  if (/^#[0-9a-f]{6}$/.test(trimmed)) {
    return trimmed;
  }
  if (/^#[0-9a-f]{3}$/.test(trimmed)) {
    return `#${[...trimmed.slice(1)].map((digit) => digit + digit).join("")}`;
  }
  return null;
}

const GROUP_ATTRIBUTES = ["data-scryer-if", "data-scryer-unless"] as const;
const RECT_ATTRIBUTES = ["x", "y", "width", "height", "rx", "fill", "fill-opacity"] as const;
const TEXT_ATTRIBUTES = [
  "x",
  "y",
  "font-family",
  "font-weight",
  "font-size",
  "fill",
  "text-anchor",
  "letter-spacing",
] as const;

const BADGE_PATTERN = /^<g\b([^>]*)>\s*<rect\b([^>]*?)\/>\s*<text\b([^>]*)>([^<]*)<\/text>\s*<\/g>/;

function parseBadge(match: RegExpExecArray, key: string): OverlayElement | null {
  const [, groupSource, rectSource, textSource, textContent] = match;
  return parseBadgeParts(
    groupSource ?? "",
    rectSource ?? "",
    textSource ?? "",
    textContent ?? "",
    key,
  );
}

function parseBadgeParts(
  groupSource: string,
  rectSource: string,
  textSource: string,
  textContent: string,
  key: string,
): OverlayElement | null {
  const group = parseAttributes(groupSource ?? "");
  const rect = parseAttributes(rectSource ?? "");
  const text = parseAttributes(textSource ?? "");
  if (!group || !rect || !text) {
    return null;
  }
  if (
    !onlyKnown(group, GROUP_ATTRIBUTES) ||
    !onlyKnown(rect, RECT_ATTRIBUTES) ||
    !onlyKnown(text, TEXT_ATTRIBUTES)
  ) {
    return null;
  }
  const anchor = text.get("text-anchor") ?? "start";
  if (anchor !== "start" && anchor !== "middle" && anchor !== "end") {
    return null;
  }
  const x = parseNumber(rect.get("x"), 0);
  const y = parseNumber(rect.get("y"), 0);
  const width = parseNumber(rect.get("width"), Number.NaN);
  const height = parseNumber(rect.get("height"), Number.NaN);
  const radius = parseNumber(rect.get("rx"), 0);
  const backgroundOpacity = parseNumber(rect.get("fill-opacity"), 1);
  const fontSize = parseNumber(text.get("font-size"), 16);
  const letterSpacing = parseNumber(text.get("letter-spacing"), 0);
  const background = parseHexColor(rect.get("fill"), "#000000");
  const textColor = parseHexColor(text.get("fill"), "#000000");
  const values = [x, y, width, height, radius, backgroundOpacity, fontSize, letterSpacing];
  if (values.some((value) => value === null || Number.isNaN(value)) || !background || !textColor) {
    return null;
  }
  return {
    key,
    showWhen: group.get("data-scryer-if") ?? "",
    hideWhen: group.get("data-scryer-unless") ?? "",
    text: unescapeXml((textContent ?? "").trim()),
    x: x as number,
    y: y as number,
    width: width as number,
    height: height as number,
    fontSize: fontSize as number,
    radius: radius as number,
    textColor,
    background,
    backgroundOpacity: backgroundOpacity as number,
    align: anchor,
    letterSpacing: letterSpacing as number,
    ratings: null,
  };
}

const STACK_PATTERN =
  /^<g\s+data-scryer-stack="(down|up|right|left)"\s+data-scryer-step="([^"]*)"\s*>([\s\S]*?)<\/g>\s*<\/g>/;
const RATING_TILE_PATTERN =
  /<g data-scryer-if="rating_([a-z_]+)">\s*<rect\b([^>]*?)\/>\s*<use\b[^>]*?\/>\s*<text\b([^>]*)>/g;

function normalizeMarkup(markup: string): string {
  return markup.replace(/\s+/g, " ").replace(/>\s+</g, "><").trim();
}

/**
 * A ratings badge, accepted only when writing it back out reproduces the
 * markup exactly: anything hand-edited stays with the SVG editor.
 */
function parseRatings(match: RegExpExecArray, key: string): OverlayElement | null {
  const [source, direction, stepSource, inner] = match;
  const tiles = [...(inner ?? "").matchAll(RATING_TILE_PATTERN)];
  const first = tiles[0];
  if (!first) {
    return null;
  }
  const tile = parseBadgeParts("", first[2] ?? "", first[3] ?? "", "", key);
  const step = Number(stepSource);
  if (!tile || !Number.isFinite(step)) {
    return null;
  }
  const sources: RatingSourceToken[] = [];
  for (const [, token] of tiles) {
    const known = RATING_SOURCES.find((candidate) => candidate.token === token);
    if (!known) {
      return null;
    }
    sources.push(known.token);
  }
  const stackDirection = direction as OverlayStackDirection;
  const along = stackDirection === "down" || stackDirection === "up" ? tile.height : tile.width;
  for (const layout of ["above", "beside"] as const) {
    const ratings: OverlayRatings = {
      sources,
      direction: stackDirection,
      gap: step - along,
      layout,
    };
    const element: OverlayElement = {
      ...tile,
      text: "",
      align: "middle",
      ratings,
    };
    if (normalizeMarkup(serializeRatings(element, ratings)) === normalizeMarkup(source ?? "")) {
      return element;
    }
  }
  return null;
}

const ROOT_PATTERN = /^\s*(?:<\?xml[^>]*\?>\s*)?<svg\b([^>]*)>([\s\S]*)<\/svg>\s*$/;

/**
 * Badges in a template, or null when the template uses anything the visual
 * editor cannot represent faithfully. `makeKey` supplies each badge's key.
 */
export function parseOverlay(svg: string, makeKey: () => string): OverlayElement[] | null {
  const root = ROOT_PATTERN.exec(svg);
  if (!root) {
    return null;
  }
  const rootAttributes = parseAttributes(root[1] ?? "");
  if (
    !rootAttributes ||
    rootAttributes
      .get("viewBox")
      ?.trim()
      .split(/[\s,]+/)
      .join(" ") !== `0 0 ${OVERLAY_CANVAS_WIDTH} ${OVERLAY_CANVAS_HEIGHT}` ||
    rootAttributes.get("data-scryer-version")?.trim() !== "1" ||
    !onlyKnown(rootAttributes, ["xmlns", "viewBox", "data-scryer-version"])
  ) {
    return null;
  }
  let body = (root[2] ?? "").replace(/<!--[\s\S]*?-->/g, "").trim();
  const elements: OverlayElement[] = [];
  while (body.length > 0) {
    const stack = STACK_PATTERN.exec(body);
    if (stack) {
      const element = parseRatings(stack, makeKey());
      if (!element) {
        return null;
      }
      elements.push(element);
      body = body.slice(stack[0].length).trim();
      continue;
    }
    const match = BADGE_PATTERN.exec(body);
    if (!match) {
      return null;
    }
    const element = parseBadge(match, makeKey());
    if (!element) {
      return null;
    }
    elements.push(element);
    body = body.slice(match[0].length).trim();
  }
  return elements;
}

/** Fields a badge's Show when / Hide when can be set from the editor. */
export const CONDITION_FIELDS = [
  "resolution",
  "hdr",
  "audio_codec",
  "audio_channels",
  "edition",
  "series_status",
  "video_codec",
  "source",
] as const;
export type ConditionField = (typeof CONDITION_FIELDS)[number];

/**
 * A badge's conditions in the shape the editor offers: one field, shown for
 * all of its values or a chosen few, and hidden for some. Hide wins over show,
 * exactly as `data-scryer-unless` wins over `data-scryer-if`.
 */
export type BadgeCondition = {
  /** Null: the badge has no conditions and always shows. */
  field: ConditionField | null;
  /** `"all"`: any value of the field (the field is not empty). */
  show: "all" | string[];
  hide: string[];
};

function isConditionField(value: string): value is ConditionField {
  return (CONDITION_FIELDS as readonly string[]).includes(value);
}

/** One clause, `field` or `field=a|b`; null for anything richer. */
function parseClause(raw: string): { field: ConditionField; values: string[] | null } | null {
  const clause = raw.trim();
  if (clause.includes(";") || clause.includes("!=")) {
    return null;
  }
  const [field, options] = clause.includes("=")
    ? (clause.split("=", 2) as [string, string])
    : [clause, null];
  const name = field.trim();
  if (!isConditionField(name)) {
    return null;
  }
  if (options === null) {
    return { field: name, values: null };
  }
  const values = options
    .split("|")
    .map((value) => value.trim())
    .filter(Boolean);
  return values.length > 0 ? { field: name, values } : null;
}

/**
 * The editor's view of a badge's conditions, or null when they use syntax the
 * dropdowns cannot show (several clauses, `!=`, two different fields, or a
 * label field). Those stay editable as text.
 */
export function parseBadgeCondition(showWhen: string, hideWhen: string): BadgeCondition | null {
  const show = showWhen.trim();
  const hide = hideWhen.trim();
  if (!show && !hide) {
    return { field: null, show: "all", hide: [] };
  }
  // "Always show, but hide for X" has no dropdown form: "All" means the field
  // has a value, which would quietly hide the badge when it is empty.
  if (!show) {
    return null;
  }
  const showClause = show ? parseClause(show) : null;
  const hideClause = hide ? parseClause(hide) : null;
  if ((show && !showClause) || (hide && !hideClause)) {
    return null;
  }
  if (hideClause && hideClause.values === null) {
    return null;
  }
  const field = showClause?.field ?? hideClause?.field ?? null;
  if (showClause && hideClause && showClause.field !== hideClause.field) {
    return null;
  }
  return {
    field,
    show: showClause?.values ?? "all",
    hide: hideClause?.values ?? [],
  };
}

/** The `showWhen` / `hideWhen` strings for a condition chosen in the editor. */
export function formatBadgeCondition(condition: BadgeCondition): {
  showWhen: string;
  hideWhen: string;
} {
  if (condition.field === null) {
    return { showWhen: "", hideWhen: "" };
  }
  const showWhen =
    condition.show === "all" || condition.show.length === 0
      ? condition.field
      : `${condition.field}=${condition.show.join("|")}`;
  const hideWhen =
    condition.hide.length > 0 ? `${condition.field}=${condition.hide.join("|")}` : "";
  return { showWhen, hideWhen };
}

/**
 * The condition value for an edition name, matching the server's
 * `edition_token`: `Director's Cut` becomes `directors_cut`.
 */
export function editionToken(edition: string): string {
  let out = "";
  let pendingSeparator = false;
  for (const ch of edition.trim()) {
    if (/^[A-Za-z0-9]$/.test(ch)) {
      if (pendingSeparator && out.length > 0) {
        out += "_";
      }
      pendingSeparator = false;
      out += ch.toLowerCase();
    } else if (ch === "'" || ch === "’") {
      continue;
    } else {
      pendingSeparator = true;
    }
  }
  return out;
}
