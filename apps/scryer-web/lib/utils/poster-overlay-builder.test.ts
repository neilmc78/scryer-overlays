import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  BADGE_KINDS,
  OVERLAY_CANVAS_HEIGHT,
  OVERLAY_CANVAS_WIDTH,
  badgeKind,
  clampElement,
  newElement,
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
  elements[1] = { ...elements[1]!, align: "start", letterSpacing: 3, radius: 0 };
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
