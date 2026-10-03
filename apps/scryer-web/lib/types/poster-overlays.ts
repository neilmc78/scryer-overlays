export type PosterOverlaySettings = {
  parallelism: number;
  reconcileIntervalSeconds: number;
};

export type PosterOverlayLibrary = {
  libraryId: string;
  libraryName: string;
  facet: string;
  enabled: boolean;
  /** Null when the library uses the built-in template. */
  templateId: string | null;
};

export type PosterOverlayTemplate = {
  id: string;
  name: string;
  svg: string;
  contentHash: string;
  createdAt: string;
  updatedAt: string;
};

export type PosterOverlayCounts = {
  enabledTitles: number;
  rendered: number;
  failed: number;
  noArtwork: number;
};

export type PosterOverlayOverview = {
  settings: PosterOverlaySettings;
  libraries: PosterOverlayLibrary[];
  templates: PosterOverlayTemplate[];
  counts: PosterOverlayCounts;
  builtinTemplate: string;
  templateSpecVersion: number;
  templateFields: string[];
  sampleOptions: PosterOverlaySampleOptions;
};

/** One value of a badge field: the token conditions match, and its label. */
export type PosterOverlaySampleOption = {
  token: string;
  label: string;
};

export type PosterOverlaySampleOptions = {
  resolutions: PosterOverlaySampleOption[];
  hdr: PosterOverlaySampleOption[];
  audio: PosterOverlaySampleOption[];
  audioChannels: string[];
  seriesStatus: PosterOverlaySampleOption[];
  videoCodec: PosterOverlaySampleOption[];
  source: PosterOverlaySampleOption[];
};

/** Values the template preview shows. Empty strings leave a field unset. */
export type PosterOverlaySample = {
  resolution: string;
  hdr: string;
  audio: string;
  audioChannels: string;
  edition: string;
  seriesStatus: string;
  videoCodec: string;
  source: string;
};

/** The kind of library the preview poster comes from. */
export type PosterOverlayPreviewFacet = "movie" | "series" | "anime";

export type PosterOverlayPreviewState = {
  /** `data:` URL of the last rendered preview. */
  image: string | null;
  /** True when drawn on a poster from the library. */
  libraryPoster: boolean;
  /** The library title the preview is drawn on. */
  posterTitleName: string | null;
  error: string | null;
  loading: boolean;
};

export type PosterOverlayTemplateDraft = {
  /** Empty for a new template. */
  id: string;
  name: string;
  svg: string;
};

export type PosterOverlayTemplateValidation = {
  valid: boolean;
  error: string | null;
};
