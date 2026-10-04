import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  BADGE_KINDS,
  OVERLAY_CANVAS_HEIGHT,
  OVERLAY_CANVAS_WIDTH,
  badgeKind,
  clampElement,
  editionToken,
  elementBounds,
  formatBadgeCondition,
  newElement,
  parseBadgeCondition,
  parseOverlay,
  serializeOverlay,
  type OverlayElement,
} from "./poster-overlay-builder.ts";

function keys(): () => string {
  let next = 0;
  return () => `k${next++}`;
}

function withoutKeys(elements: OverlayElement[] | null) {
  return elements?.map(({ key: _key, ...rest }) => rest) ?? null;
}

const BUILTIN = readFileSync(
  new URL(
    "../../../../crates/scryer-infrastructure-library/assets/overlays/default.svg",
    import.meta.url,
  ),
  "utf8",
);

test("the built-in template opens in the visual editor", () => {
  const elements = parseOverlay(BUILTIN, keys());
  assert.ok(elements, "built-in template must be editable");
  assert.deepEqual(
    elements.map((element) => badgeKind(element)),
    ["resolution", "hdr", "hdr", "edition", "audio"],
  );
  const resolution = elements[0]!;
  assert.deepEqual(
    [resolution.x, resolution.y, resolution.width, resolution.height, resolution.fontSize],
    [40, 40, 270, 110, 62],
  );
  assert.equal(resolution.text, "{{resolution_label}}");
  const edition = elements[3]!;
  assert.equal(edition.radius, 0, "a box without rx has square corners");
  assert.equal(edition.letterSpacing, 4);
  assert.equal(elements[4]!.hideWhen, "audio_codec=other");
});

test("serialising and parsing again preserves every badge", () => {
  const make = keys();
  const elements = BADGE_KINDS.map((kind) => newElement(kind, make()));
  elements[1] = {
    ...elements[1]!,
    align: "start",
    letterSpacing: 3,
    radius: 0,
  };
  elements[2] = { ...elements[2]!, align: "end", backgroundOpacity: 0 };
  const svg = serializeOverlay(elements);
  assert.deepEqual(withoutKeys(parseOverlay(svg, keys())), withoutKeys(elements));
});

test("text with XML special characters survives a round trip", () => {
  const element = { ...newElement("custom", "a"), text: `R&D <cut> "final"` };
  const parsed = parseOverlay(serializeOverlay([element]), keys());
  assert.equal(parsed?.[0]?.text, `R&D <cut> "final"`);
});

test("templates the visual editor cannot represent are left to the SVG editor", () => {
  const header = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1000 1500" data-scryer-version="1">`;
  const badge = `<g data-scryer-if="resolution"><rect x="0" y="0" width="10" height="10" fill="#000"/><text x="5" y="5" fill="#fff">{{resolution_label}}</text></g>`;
  assert.ok(parseOverlay(`${header}${badge}</svg>`, keys()), "baseline is editable");
  for (const unsupported of [
    `${header}<circle cx="5" cy="5" r="5"/></svg>`,
    `${header}${badge}<path d="M0 0"/></svg>`,
    `${header}${badge.replace('fill="#000"', 'fill="url(#g)"')}</svg>`,
    `${header}${badge.replace("<rect", '<rect stroke="#fff"')}</svg>`,
    `${header}${badge.replace("{{resolution_label}}", "<tspan>4K</tspan>")}</svg>`,
    badge,
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 150" data-scryer-version="1">${badge}</svg>`,
  ]) {
    assert.equal(parseOverlay(unsupported, keys()), null, unsupported);
  }
});

test("badges are kept inside the poster with usable sizes", () => {
  const clamped = clampElement({
    ...newElement("custom", "a"),
    x: 980,
    y: -50,
    width: 300,
    height: 0,
    fontSize: 9000,
    radius: 500,
    backgroundOpacity: 3,
  });
  assert.equal(clamped.x, OVERLAY_CANVAS_WIDTH - 300);
  assert.equal(clamped.y, 0);
  assert.equal(clamped.height, 1);
  assert.ok(clamped.y + clamped.height <= OVERLAY_CANVAS_HEIGHT);
  assert.equal(clamped.fontSize, 400);
  assert.equal(clamped.radius, 0.5);
  assert.equal(clamped.backgroundOpacity, 1);
});

test("badge conditions round-trip through the dropdown model", () => {
  for (const [showWhen, hideWhen, expected] of [
    ["", "", { field: null, show: "all", hide: [] }],
    ["resolution", "", { field: "resolution", show: "all", hide: [] }],
    ["resolution", "resolution=2160p", { field: "resolution", show: "all", hide: ["2160p"] }],
    [
      "hdr=hdr10plus|hdr10|hlg",
      "",
      { field: "hdr", show: ["hdr10plus", "hdr10", "hlg"], hide: [] },
    ],
    ["audio_codec", "audio_codec=other", { field: "audio_codec", show: "all", hide: ["other"] }],
  ] as const) {
    const parsed = parseBadgeCondition(showWhen, hideWhen);
    assert.deepEqual(parsed, expected, `${showWhen} / ${hideWhen}`);
    assert.deepEqual(formatBadgeCondition(parsed!), { showWhen, hideWhen });
  }
});

test("hide is written as a separate condition so it overrides show", () => {
  assert.deepEqual(formatBadgeCondition({ field: "resolution", show: "all", hide: ["2160p"] }), {
    showWhen: "resolution",
    hideWhen: "resolution=2160p",
  });
  assert.deepEqual(formatBadgeCondition({ field: "hdr", show: ["dv", "hdr10"], hide: [] }), {
    showWhen: "hdr=dv|hdr10",
    hideWhen: "",
  });
  // Nothing ticked under Show means every value, never "show nothing".
  assert.deepEqual(formatBadgeCondition({ field: "hdr", show: [], hide: [] }), {
    showWhen: "hdr",
    hideWhen: "",
  });
});

test("conditions the dropdowns cannot express stay as text", () => {
  for (const [showWhen, hideWhen] of [
    ["resolution=2160p;hdr=dv", ""],
    ["hdr!=sdr", ""],
    ["resolution", "hdr=sdr"],
    ["resolution_label=4K", ""],
    ["", "resolution"],
    ["", "edition=extended"],
  ]) {
    assert.equal(parseBadgeCondition(showWhen!, hideWhen!), null, `${showWhen} / ${hideWhen}`);
  }
});

test("edition names become the same condition values the server uses", () => {
  assert.equal(editionToken("Director's Cut"), "directors_cut");
  assert.equal(editionToken("  Extended  Edition "), "extended_edition");
  assert.equal(editionToken("IMAX: Enhanced"), "imax_enhanced");
  assert.equal(editionToken("Director’s Cut"), "directors_cut");
});

test("series status badges open in the dropdowns", () => {
  const status = newElement("status", "s");
  assert.equal(badgeKind(status), "status");
  assert.deepEqual(parseBadgeCondition(status.showWhen, status.hideWhen), {
    field: "series_status",
    show: "all",
    hide: [],
  });
  assert.deepEqual(
    formatBadgeCondition({
      field: "series_status",
      show: ["ended", "canceled"],
      hide: [],
    }),
    { showWhen: "series_status=ended|canceled", hideWhen: "" },
  );
});

test("a ratings badge writes one tile per source and opens again", () => {
  const ratings = {
    ...newElement("ratings", "r"),
    ratings: {
      sources: ["imdb", "metacritic_user"],
      direction: "right",
      gap: 12,
      layout: "beside",
    },
  } satisfies OverlayElement;
  const svg = serializeOverlay([ratings]);
  assert.match(svg, /data-scryer-stack="right" data-scryer-step="212"/);
  assert.match(
    svg,
    /data-scryer-if="rating_imdb"[\s\S]*#scryer-logo-imdb[\s\S]*\{\{rating_imdb\}\}/,
  );
  // The user score shares the Metacritic logo but has its own score.
  assert.match(svg, /#scryer-logo-metacritic"[\s\S]*\{\{rating_metacritic_user\}\}/);
  const [parsed] = parseOverlay(svg, keys()) ?? [];
  assert.deepEqual(withoutKeys(parsed ? [parsed] : null), withoutKeys([ratings]));
});

test("a hand-edited ratings badge is left to the SVG editor", () => {
  const svg = serializeOverlay([newElement("ratings", "r")]);
  for (const edited of [
    svg.replace('fill-opacity="0.82"/>', 'fill-opacity="0.82" stroke="#fff"/>'),
    svg.replace("rating_tmdb", "rating_netflix"),
    svg.replace(/width="200" height="200"/, 'width="210" height="200"'),
  ]) {
    assert.notEqual(edited, svg);
    assert.equal(parseOverlay(edited, keys()), null, edited);
  }
});

test("a ratings badge covers every tile it can draw", () => {
  const element = newElement("ratings", "r");
  assert.deepEqual(elementBounds(element), {
    x: 40,
    y: 300,
    width: 200,
    height: 640,
  });
  assert.deepEqual(
    elementBounds({
      ...element,
      ratings: { ...element.ratings!, direction: "left" },
    }),
    { x: -400, y: 300, width: 640, height: 200 },
  );
});
