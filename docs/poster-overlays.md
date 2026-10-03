# Poster overlays

Scryer can draw quality badges (resolution, HDR, video codec, source, audio,
edition, series status) over title posters. Badges come from each title's own
media files and the metadata Scryer already stores; nothing is looked up for
them. Overlays are enabled per library under Settings > Poster overlays.

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
| `video_codec` | `h266`, `av1`, `h265`, `vp9`, `h264`, `vc1`, `mpeg4`, `xvid`, `divx`, `mpeg2` |
| `video_codec_label` | `H.266`, `AV1`, `H.265`, `VP9`, `H.264`, `VC-1`, `MPEG-4`, `XVID`, `DIVX`, `MPEG-2` |
| `source` | `remux`, `brdisk`, `bluray`, `webdl`, `webrip`, `hdtv`, `dvd`, `dvdscr`, `telecine`, `telesync`, `cam`, `workprint` |
| `source_label` | `REMUX`, `BR-DISK`, `BLURAY`, `WEB-DL`, `WEBRIP`, `HDTV`, `DVD`, `DVDSCR`, `TELECINE`, `TELESYNC`, `CAM`, `WORKPRINT` |
| `edition` | Every edition's lowercase slug, comma separated, for example `directors_cut,theatrical` |
| `edition_label` | Every edition's upper-case text, joined with ` / `, for example `DIRECTOR'S CUT / THEATRICAL` |
| `series_status` | `continuing`, `upcoming`, `ended`, `canceled` |
| `series_status_label` | `CONTINUING`, `UPCOMING`, `ENDED`, `CANCELED` |

How the values are resolved:

- **Resolution** comes from the probed frame size. A scope-cropped 3840x1600
  encode counts as `2160p`. The release-name resolution is used only when the
  file has not been probed.
- **HDR** is `sdr` only for probed files. For an unprobed file it is empty.
  A Dolby Vision file with an HDR10 fallback layer reports `dv`.
- **Video codec** is the probed codec, else the codec in the release name.
  Both go through the release parser's vocabulary, so `hevc`, `x265` and
  `H.265` are all `h265`, and `avc`, `x264` and `H.264` are all `h264`.
- **Source** is the release source parsed from the file's release name.
- **Series and multi-file titles** take the best value of each quality field
  (resolution, HDR, video codec, source, audio) across their primary files:
  the highest resolution, the most modern codec, the best source. Additional
  versions beside the primary file do not count for quality, unless a title
  has no primary file.
- **Edition** lists every distinct edition across primary and additional
  files, primary first. A file's edition is its parsed edition, else the
  Plex/Radarr `{edition-Name}` tag in its file name. A folder holding a
  director's cut and a theatrical cut therefore reports both.
- **Series status** comes from the title's stored metadata status, for series
  and anime only. TVDB-style statuses (`Continuing`, `Ended`, `Upcoming`) and
  TMDB's (`Returning Series`, `Planned`, `Pilot`, `In Production`, `Ended`,
  `Canceled`) fold onto the four tokens.

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

`edition` and `edition_label` hold several values. For them, `=` holds when
any edition is listed, and `!=` holds when no edition is.

Values are checked against each field's tokens above, or its labels for a
`_label` field. A condition naming a value the field can never take, such as
`resolution=4K` (the token is `2160p`), is rejected rather than saved as a
badge that never shows. `audio_channels` also accepts `Nch`, and `edition`
values must be slugs.

```xml
<g data-scryer-if="hdr=dv|hdr10plus">...</g>
<g data-scryer-if="resolution=2160p;hdr!=sdr">...</g>
<g data-scryer-if="audio_codec" data-scryer-unless="audio_codec=other">...</g>
<g data-scryer-if="edition=directors_cut|extended">...</g>
```

### Restrictions

A template is rejected if it:

- contains `<image>`, `<foreignObject>`, `<script>` or `<feImage>`;
- contains a `DOCTYPE`;
- is larger than 256 KiB;
- references an unknown field;
- compares a field against a value it can never take.

The settings page can check a template before it is saved.

### Versioning

`data-scryer-version` names the contract version a template was written for.
Version 1 is the format described here.

- New fields and tokens may be added within a version. A future version may
  add clause forms or attributes.
- Changing the meaning of anything in an older version requires a new version
  number.
- The server keeps accepting every version up to its own.

## Rendering and rebuilds

### Files and output

- The pristine original is fetched once and kept at
  `<data_dir>/overlays/originals/<title_id>`. For TMDB posters it is the
  780px-wide rendition.
- The original comes from the title's cached poster source. When the image
  cache is empty (after a backup restore, or before artwork is cached) the
  poster URL stored on the title is used instead.
- A title with no poster artwork at all is counted on the settings page and
  logged by the reconcile pass. It renders once metadata provides a poster.
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
  replaces the stored original and the poster is re-rendered. Series status
  changes arrive with metadata refreshes, which raise no event, so they are
  picked up here.

Rendering runs on a dedicated thread pool, never on the async runtime. Its
parallelism is configurable (default 3) and bounds memory during
library-wide rebuilds.

### Serving

- Clients load title posters through Scryer's image proxy,
  `/images/media/{token}/{variant}`. When a token's source is a title's
  poster and that title's library has overlays enabled, the rendered overlay
  is served in its place. Everything else is proxied unchanged.
- `/images/titles/{title_id}/poster/{variant}` serves overlays the same way.
- Overlay responses carry an `overlay:<output_hash>` ETag and are revalidated
  on every load, so a re-render shows on the next refresh.
- When no library has overlays enabled, the overlay lookup is skipped.

### Reverting

"Revert all to originals":

- disables overlays in every library;
- serves every stored original again;
- removes rendered outputs and their state.

Stored originals are kept.

## Template editor

The settings page edits templates visually. The result is an ordinary
template in the format above.

- Badges are added from presets (resolution, HDR, video codec, source, audio,
  edition, series status, free text). Each has a position, size, font size,
  colours, background opacity, corner radius and alignment, and can be
  dragged on the preview.
- Each badge picks one field, the values to show for (`All` means any value)
  and the values to hide for. Hide wins, as `data-scryer-unless` does.
  Options are labelled as the badge prints them, for example `4K (2160p)`.
- A template using SVG the visual editor cannot represent stays editable as
  text and is never rewritten.
- The preview is rendered on the server with chosen sample values, on a
  random stored original from a chosen kind of library (movies, series or
  anime). The same poster is kept between edits until Shuffle. Nothing is
  stored.

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
