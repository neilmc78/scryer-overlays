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
};

export type PosterOverlayOverview = {
  settings: PosterOverlaySettings;
  libraries: PosterOverlayLibrary[];
  templates: PosterOverlayTemplate[];
  counts: PosterOverlayCounts;
  builtinTemplate: string;
  templateSpecVersion: number;
  templateFields: string[];
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
