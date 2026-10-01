# Poster overlays

Scryer can draw quality badges (resolution, HDR, audio, edition) over title
posters. Badges come only from each title's own media files. Nothing is looked
up externally. Overlays are enabled per library under Settings > Poster
overlays.

This document is the public contract for overlay templates. It also records
how rendering, rebuilds and the planned Plex integration behave.

## Template format

A template is an SVG document. Scryer resolves its placeholders and
conditions, renders it with resvg, and composites the result over the poster.

### Root element

```xml
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1000 1500" data-scryer-version="1">
  ...
</svg>
```

- The root element must be `<svg>` and must declare `data-scryer-version`.
- The template is rejected if the version is newer than the server supports.
  The settings page shows the supported version.
- `data-scryer-*` attributes are removed before rendering.

### Positioning

- Coordinates are in `viewBox` units.
- The viewBox is scaled uniformly to fit the poster and centred (SVG
  `xMidYMid meet`).
- TMDB posters are 2:3, so a `0 0 1000 1500` viewBox maps edge to edge.
- Posters with another aspect ratio get equal margins on the long axis.
- Text uses the embedded Inter Bold font. `font-family` is ignored: every
  family resolves to it, and no system fonts are read.

### Fields

Each field resolves to a token, or to an empty string when it is unknown.
Tokens are stable: new tokens may be added, but an existing token never
changes meaning.

| Field | Values |
|---|---|
| `resolution` | `2160p`, `1080p`, `720p`, `sd` |
| `resolution_label` | `4K`, `1080P`, `720P`, `SD` |
| `hdr` | `dv`, `hdr10plus`, `hdr10`, `hlg`, `sdr` |
| `hdr_label` | `DOLBY VISION`, `HDR10+`, `HDR10`, `HLG`, `SDR` |
| `audio_codec` | `dtsx`, `truehd_atmos`, `ddp_atmos`, `dtshd_ma`, `truehd`, `pcm`, `flac`, `dtshd_hra`, `dts`, `ddp`, `dd`, `opus`, `aac`, `mp3`, `other` |
| `audio_label` | Display text for `audio_codec`, for example `TRUEHD ATMOS` |
| `audio_channels` | `1.0`, `2.0`, `2.1`, `5.1`, `6.1`, `7.1`, or `Nch` |
| `edition` | Lowercase slug, for example `directors_cut` |
| `edition_label` | Upper-case edition text, for example `DIRECTOR'S CUT` |

How the values are resolved:

- **Resolution** comes from the probed frame size. A scope-cropped 3840x1600
  encode counts as `2160p`. The release-name resolution is used only when the
  file has not been probed.
- **HDR** is `sdr` only for probed files. For an unprobed file it is empty.
  A Dolby Vision file with an HDR10 fallback layer reports `dv`.
- **Series and multi-file titles** take the best value of each field across
  all their primary files.
- **Edition** is set only when every file that names one agrees.

### Placeholders

`{{field}}` is replaced in text and attribute values. Values are XML-escaped
when inserted. An unknown field name is an error.

```xml
<text x="155" y="118">{{resolution_label}}</text>
```

### Conditions

Any element may carry these attributes:

- `data-scryer-if`: the element and its children render only when the
  condition holds.
- `data-scryer-unless`: the element and its children render only when the
  condition does not hold.

A condition is one or more clauses separated by `;`, and every clause must
hold. Three clause forms are accepted:

| Clause | Holds when |
|---|---|
| `field` | the field is not empty |
| `field=a\|b` | the field equals any listed value |
| `field!=a\|b` | the field equals none of the listed values |

```xml
<g data-scryer-if="hdr=dv|hdr10plus">...</g>
<g data-scryer-if="resolution=2160p;hdr!=sdr">...</g>
<g data-scryer-if="audio_codec" data-scryer-unless="audio_codec=other">...</g>
```

### Restrictions

A template is rejected if it:

- contains `<image>`, `<foreignObject>`, `<script>` or `<feImage>`;
- contains a `DOCTYPE`;
- is larger than 256 KiB;
- references an unknown field.

The settings page can check a template before it is saved.

### Versioning

`data-scryer-version` names the contract version a template was written for.
Version 1 is the format described here.

- A future version may add fields, clause forms or attributes.
- Changing the meaning of anything in an older version requires a new version
  number.
- The server keeps accepting every version up to its own.

## Rendering and rebuilds

### Files and output

- The pristine original is fetched once and kept at
  `<data_dir>/overlays/originals/<title_id>`. For TMDB posters it is the
  780px-wide rendition.
- Rendered posters are stored as JPEG in
  `<data_dir>/overlays/output/<title_id>/`, at full size plus the `w250` and
  `w70` sizes the web UI uses.
- Each output carries a JPEG comment marker, `scryer-overlay:v1:<input_hash>`.
  The marker is written without re-encoding the image.
- An image carrying the marker is never accepted as an original.
- The stored poster images Scryer already keeps are never modified.

### When a poster rebuilds

- `input_hash = blake3(original_hash, template_version, resolved field values)`.
- `template_version` combines the contract version, a renderer revision and
  the template's content hash, so editing a template rebuilds its posters.
- A poster is re-rendered only when `input_hash` changes. That one check
  covers upgrades, template edits and new upstream artwork.

Rebuilds are triggered by:

- **Media events:** `MediaFileAnalyzed`, `MediaFileImported`,
  `MediaFileUpgraded`, `MediaFileDeleted`, `MediaFileRestored` and
  `TitleDeleted`. Probe data arrives with `MediaFileAnalyzed`.
- **The periodic reconcile:** every six hours by default. It also adopts new
  upstream artwork: when the title's poster source changes, the new image
  replaces the stored original and the poster is re-rendered.

Rendering runs on a dedicated thread pool, never on the async runtime. Its
parallelism is configurable (default 3) and bounds memory during
library-wide rebuilds.

### Reverting

"Revert all to originals":

- disables overlays in every library;
- serves every stored original again;
- removes rendered outputs and their state.

Stored originals are kept.

## Plex (planned, not yet built)

A future setting will push rendered posters to Plex:

1. Upload the poster to the Plex item (`POST /library/metadata/{ratingKey}/posters`).
2. Lock the poster field so a metadata refresh does not revert it.
3. Delete the upload it replaces, so Plex's metadata folder does not grow.

The Plex item is found through Scryer's existing media-server item mappings.
The reconcile pass will compare the poster Plex serves with the last pushed
output. A poster without the marker means someone else changed it; that image
becomes the new original.

Scryer must be the only tool drawing overlays on these posters. Disable
Kometa or Agregarr overlays for the same libraries before enabling the Plex
push. Two overlay owners would stack badges on top of each other or overwrite
each other's work.

## Attribution

- The embedded font is Inter by The Inter Project Authors, licensed under the
  SIL Open Font License 1.1. The licence ships beside the font in
  `crates/scryer-infrastructure-library/assets/overlays/Inter-OFL.txt`.
- The badge designs, template format and rendering code are original to
  Scryer. The overlay concept follows Kometa and Agregarr, but no code or
  assets are taken from either.
