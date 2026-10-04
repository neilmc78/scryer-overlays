# Poster overlays

Scryer can draw quality badges (resolution, HDR, video codec, source, audio,
edition, series status) and rating badges over title posters. Badges come
from each title's own media files and the metadata Scryer already stores;
nothing is looked up for them. Overlays are enabled per library under
Settings > Poster overlays, and can also be pushed to Plex.

This document is the public contract for overlay templates. It also records
how rendering, rebuilds and the Plex push behave.

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
| `rating_imdb`, `rating_rottentomatoes`, `rating_popcornmeter`, `rating_metacritic`, `rating_metacritic_user`, `rating_letterboxd`, `rating_tmdb`, `rating_trakt`, `rating_mdblist` | The title's score from that source as the web UI writes it, for example `7.8`, `74%` or `81` |

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
- **Ratings** come from the scores stored with the title's metadata. Rotten
  Tomatoes and Popcornmeter are percentages, Metacritic and its user score are
  out of 100, and the other sources show their own value to one decimal place.
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

### Logos

`<use href="#scryer-logo-NAME" x="..." y="..." width="..." height="..."/>`
draws a bundled logo scaled to fit the box, keeping its aspect ratio. The
logos are `imdb`, `rottentomatoes`, `popcornmeter`, `metacritic`,
`letterboxd`, `tmdb`, `trakt` and `mdblist`; Metacritic's user score uses the
`metacritic` logo. The renderer adds only the logos a poster actually draws.

```xml
<g data-scryer-if="rating_imdb">
  <use href="#scryer-logo-imdb" x="56" y="316" width="168" height="92"/>
  <text x="140" y="465">{{rating_imdb}}</text>
</g>
```

### Stacks

A group with `data-scryer-stack` and `data-scryer-step` lays out its children
in a row or column. Children hidden by their conditions take no slot, so a
title with no score from one source shows no gap.

- `data-scryer-stack` is `down`, `up`, `right` or `left`.
- `data-scryer-step` is the distance between slots, in viewBox units.
- Write every child at the first slot's position. The nth child shown is
  moved `n - 1` steps along.

```xml
<g data-scryer-stack="down" data-scryer-step="220">
  <g data-scryer-if="rating_rottentomatoes">...</g>
  <g data-scryer-if="rating_imdb">...</g>
</g>
```

### Fitted text

A `<text>` with `data-scryer-fit` is sized to a box instead of overflowing
it.

- `data-scryer-fit` is `x y width height`, in viewBox units.
- `data-scryer-fit-lines` is how many lines the text may wrap onto, 1 to 4.
  Without it the text stays on one line.
- The text's own `font-size` is the largest size used. It shrinks only as
  far as needed for its words to fit the box's width in the allowed lines,
  and the lines' height to fit the box. It never goes below 6.
- Lines break between words. The block is centred vertically in the box.
  `x` and `text-anchor` place each line horizontally as usual; the text's
  own `y` is ignored.
- The text is measured with the embedded font, as it will be drawn.
- A fitted `<text>` may contain only text and placeholders, no child
  elements.

```xml
<text x="150" font-size="44" text-anchor="middle"
      data-scryer-fit="58 115 184 110" data-scryer-fit-lines="2">{{edition_label}}</text>
```

### Restrictions

A template is rejected if it:

- contains `<image>`, `<foreignObject>`, `<script>` or `<feImage>`;
- contains a `DOCTYPE`;
- is larger than 256 KiB;
- references an unknown field;
- compares a field against a value it can never take;
- references a logo that is not bundled, or uses an id starting with
  `scryer-logo-`;
- has a stack without both attributes, an unknown direction, or a step that
  is not a positive number;
- uses `data-scryer-fit` on anything but `<text>`, with a malformed box,
  with a line limit outside 1 to 4, or on a `<text>` holding elements.

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
  picked up here, as are rating changes.

Rendering runs on a dedicated thread pool, never on the async runtime. Its
parallelism is configurable (default 3) and bounds memory during
library-wide rebuilds.

A library-wide pass logs one line when it starts, with the number of titles,
and one when it finishes, with how many posters were re-rendered, left
unchanged or failed. Nothing is logged per poster. While a pass runs, the
settings page shows how many titles it has checked and re-rendered; it reads
that from memory, not the database.

### Serving

- Clients load title posters through Scryer's image proxy,
  `/images/media/{token}/{variant}`. When a token's source is a title's
  poster and that title's library has overlays enabled, the rendered overlay
  is served in its place. Everything else is proxied unchanged.
- `/images/titles/{title_id}/poster/{variant}` serves overlays the same way.
- Overlay responses carry an `overlay:<output_hash>` ETag and are revalidated
  on every load, so a re-render shows on the next refresh.
- When no library has overlays enabled, the overlay lookup is skipped.
- A browser that cached a poster under a long-lived cache header before
  overlays were enabled keeps showing that copy until its cache expires or is
  cleared.

### Reverting

"Revert all to originals":

- disables overlays in every library;
- serves every stored original again;
- removes rendered outputs and their state;
- with the Plex push in use, uploads the stored originals back to Plex and
  unlocks their poster fields (see below).

Stored originals are kept.

## Template editor

The settings page edits templates visually. The result is an ordinary
template in the format above.

- Badges are added from presets (resolution, HDR, video codec, source, audio,
  edition, series status, ratings, free text). Each has a position, size, font size,
  colours, background opacity, corner radius and alignment, and can be
  dragged on the preview.
- Text fit is Fixed size, Shrink to fit, or Wrap onto up to 2 or 3 lines,
  written as `data-scryer-fit`. New badges shrink to fit; edition badges wrap
  onto 2 lines.
- Each badge picks one field, the values to show for (`All` means any value)
  and the values to hide for. Hide wins, as `data-scryer-unless` does.
  Options are labelled as the badge prints them, for example `4K (2160p)`.
- A ratings badge picks its sources from a list showing each source's logo.
  It draws one tile per source, with the logo above or beside the score, and
  stacks the tiles in a chosen direction with a chosen gap. Its box is one
  tile; the preview outlines the whole stack.
- A template using SVG the visual editor cannot represent stays editable as
  text and is never rewritten.
- The preview is rendered on the server with chosen sample values, on a
  random stored original from a chosen kind of library (movies, series or
  anime). The same poster is kept between edits until Shuffle. Ratings come
  from that title's scores, or sample scores on the placeholder. Nothing is
  stored.

## Plex

With **Push posters to Plex** on (Settings > Poster overlays > Plex), each
overlaid poster also becomes the poster of the title's Plex item.

### Which items

- Every enabled Plex connection with a selected server and a stored token
  is used. Scryer finds the server's address the way its catalog scan does:
  it asks plex.tv for the selected server, preferring an HTTPS address, and
  reuses the answer for ten minutes. The token is sent in a header.
- A title is pushed to the Plex item that Scryer's media-server catalog
  scan matched it to (the same match behind "Play on Plex" links). A title
  with no match is skipped until the scan finds one.
- Only titles in libraries with overlays enabled are pushed. Episodes and
  seasons are not.

### Pushing

- The full-size overlay is uploaded (`POST /library/metadata/{ratingKey}/posters`)
  and the poster field is locked, so Plex's metadata refreshes keep it.
- Scryer records, per connection and title, which output it uploaded and
  the item's `thumb` path straight after. A poster is uploaded again only
  when the rendered output changes, so a pass over an unchanged library
  makes one light read per title and no uploads.
- A failed push is recorded on the title, counted on the settings page and
  retried on the next pass. It never fails the render.

### Posters changed in Plex

When the item's `thumb` no longer matches the one recorded after Scryer's
upload, Scryer reads the poster Plex now shows. If it lacks the overlay
marker, someone chose another poster in Plex: Scryer leaves it alone, also
when its own overlay later changes, and counts it as "changed in Plex".
Selecting Scryer's poster in Plex again resumes pushing.

### Maintenance hours

Nothing is changed in Plex during its scheduled maintenance hours, read from
the server's `ButlerStartHour` and `ButlerEndHour` settings (rechecked every
ten minutes). Uploads and restores due in the window wait, and a pass runs
when it ends. Plex keeps these hours in the server's local time and Scryer
compares them in its own, so both should run in the same time zone.
Rendering in Scryer is not paused.

### Turning it off

Turning the push off, disabling overlays for a library, or "Revert all"
uploads the stored original back to each pushed item and unlocks its poster
field. A poster that was changed in Plex is left as it is. Scryer then
forgets the item.

### Nothing is deleted in Plex

Each upload adds a file to the item's folder in Plex's metadata. Scryer
never deletes anything on the server; Plex's own **Clean Bundles** task
(Settings > Troubleshooting) removes uploads no longer in use.

### Backups

Which item received which upload is reset on restore, so the first pass
after a restore uploads each poster again.

Scryer must be the only tool drawing overlays on these posters. Disable
Kometa or Agregarr overlays for the same libraries before enabling the Plex
push. Two overlay owners would stack badges on top of each other or overwrite
each other's work.

## Attribution

- The embedded font is Inter by The Inter Project Authors, licensed under the
  SIL Open Font License 1.1. The licence ships beside the font in
  `crates/scryer-infrastructure-library/assets/overlays/Inter-OFL.txt`.
- The rating source logos are the files the web UI already ships under
  `apps/scryer-web/public/rating-sources`, with internal ids prefixed; the
  MDBList logo, shipped there only as a small raster, is redrawn as a vector.
  They are trademarks of their owners and identify the source of each score.
- The badge designs, template format and rendering code are original to
  Scryer. The overlay concept follows Kometa and Agregarr, but no code or
  assets are taken from either.
