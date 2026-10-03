//! Badge fields: normalisation of probe data into the closed vocabulary the
//! template contract exposes, aggregation across a title's files, and the
//! hashes that decide whether a poster needs rebuilding.
//!
//! Everything here is pure policy. The vocabulary is part of the public
//! template contract (`docs/poster-overlays.md`): a token may be added, but an
//! existing token must never change meaning.

use std::collections::BTreeMap;

/// Probe and parse facts for one media file, as stored on `media_files`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OverlayMediaFacts {
    pub video_width: Option<i64>,
    pub video_height: Option<i64>,
    /// Release-name resolution (`2160p`, `1080p`, ...), used when the file has
    /// not been probed.
    pub parsed_resolution: Option<String>,
    /// `Dolby Vision`, `HDR10+`, `HDR10`, `HLG`, or absent.
    pub video_hdr_format: Option<String>,
    pub audio_codec: Option<String>,
    pub audio_profile: Option<String>,
    pub audio_channels: Option<i64>,
    pub edition: Option<String>,
}

impl OverlayMediaFacts {
    fn is_probed(&self) -> bool {
        self.video_width.is_some() || self.video_height.is_some()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum OverlayResolution {
    Sd,
    Hd720,
    Hd1080,
    Uhd2160,
}

impl OverlayResolution {
    /// Every value, best first: the options a template preview offers.
    pub const ALL: [Self; 4] = [Self::Uhd2160, Self::Hd1080, Self::Hd720, Self::Sd];

    pub fn from_token(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.token() == token)
    }

    pub fn token(self) -> &'static str {
        match self {
            Self::Uhd2160 => "2160p",
            Self::Hd1080 => "1080p",
            Self::Hd720 => "720p",
            Self::Sd => "sd",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Uhd2160 => "4K",
            Self::Hd1080 => "1080P",
            Self::Hd720 => "720P",
            Self::Sd => "SD",
        }
    }

    fn from_dimensions(width: i64, height: i64) -> Option<Self> {
        if width <= 0 && height <= 0 {
            return None;
        }
        // Width carries the class for scope and letterboxed encodes, height
        // for pillarboxed ones; either reaching a threshold is enough.
        Some(if width >= 3200 || height >= 1800 {
            Self::Uhd2160
        } else if width >= 1800 || height >= 900 {
            Self::Hd1080
        } else if width >= 1200 || height >= 600 {
            Self::Hd720
        } else {
            Self::Sd
        })
    }

    fn from_parsed(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "2160p" | "4k" | "uhd" | "4320p" => Some(Self::Uhd2160),
            "1080p" | "1080i" => Some(Self::Hd1080),
            "720p" => Some(Self::Hd720),
            "576p" | "576i" | "540p" | "480p" | "480i" | "360p" | "sd" => Some(Self::Sd),
            _ => None,
        }
    }

    fn resolve(facts: &OverlayMediaFacts) -> Option<Self> {
        match (facts.video_width, facts.video_height) {
            (None, None) => facts
                .parsed_resolution
                .as_deref()
                .and_then(Self::from_parsed),
            (width, height) => Self::from_dimensions(width.unwrap_or(0), height.unwrap_or(0))
                .or_else(|| {
                    facts
                        .parsed_resolution
                        .as_deref()
                        .and_then(Self::from_parsed)
                }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum OverlayHdr {
    Sdr,
    Hlg,
    Hdr10,
    Hdr10Plus,
    DolbyVision,
}

impl OverlayHdr {
    pub const ALL: [Self; 5] = [
        Self::DolbyVision,
        Self::Hdr10Plus,
        Self::Hdr10,
        Self::Hlg,
        Self::Sdr,
    ];

    pub fn from_token(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.token() == token)
    }

    pub fn token(self) -> &'static str {
        match self {
            Self::DolbyVision => "dv",
            Self::Hdr10Plus => "hdr10plus",
            Self::Hdr10 => "hdr10",
            Self::Hlg => "hlg",
            Self::Sdr => "sdr",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::DolbyVision => "DOLBY VISION",
            Self::Hdr10Plus => "HDR10+",
            Self::Hdr10 => "HDR10",
            Self::Hlg => "HLG",
            Self::Sdr => "SDR",
        }
    }

    fn resolve(facts: &OverlayMediaFacts) -> Option<Self> {
        match facts.video_hdr_format.as_deref().map(str::trim) {
            Some(value) if value.eq_ignore_ascii_case("dolby vision") => Some(Self::DolbyVision),
            Some(value) if value.eq_ignore_ascii_case("hdr10+") => Some(Self::Hdr10Plus),
            Some(value) if value.eq_ignore_ascii_case("hdr10") => Some(Self::Hdr10),
            Some(value) if value.eq_ignore_ascii_case("hlg") => Some(Self::Hlg),
            Some(value) if !value.is_empty() => None,
            // The probe writes no HDR format for SDR video. Without a probe we
            // cannot tell SDR from unknown.
            _ if facts.is_probed() => Some(Self::Sdr),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayAudio {
    DtsX,
    TrueHdAtmos,
    DdPlusAtmos,
    DtsHdMa,
    TrueHd,
    Pcm,
    Flac,
    DtsHdHra,
    Dts,
    DdPlus,
    Dd,
    Opus,
    Aac,
    Mp3,
    Other,
}

impl OverlayAudio {
    pub const ALL: [Self; 15] = [
        Self::DtsX,
        Self::TrueHdAtmos,
        Self::DdPlusAtmos,
        Self::DtsHdMa,
        Self::TrueHd,
        Self::Pcm,
        Self::Flac,
        Self::DtsHdHra,
        Self::Dts,
        Self::DdPlus,
        Self::Dd,
        Self::Opus,
        Self::Aac,
        Self::Mp3,
        Self::Other,
    ];

    pub fn from_token(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.token() == token)
    }

    pub fn token(self) -> &'static str {
        match self {
            Self::DtsX => "dtsx",
            Self::TrueHdAtmos => "truehd_atmos",
            Self::DdPlusAtmos => "ddp_atmos",
            Self::DtsHdMa => "dtshd_ma",
            Self::TrueHd => "truehd",
            Self::Pcm => "pcm",
            Self::Flac => "flac",
            Self::DtsHdHra => "dtshd_hra",
            Self::Dts => "dts",
            Self::DdPlus => "ddp",
            Self::Dd => "dd",
            Self::Opus => "opus",
            Self::Aac => "aac",
            Self::Mp3 => "mp3",
            Self::Other => "other",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::DtsX => "DTS:X",
            Self::TrueHdAtmos => "TRUEHD ATMOS",
            Self::DdPlusAtmos => "DD+ ATMOS",
            Self::DtsHdMa => "DTS-HD MA",
            Self::TrueHd => "TRUEHD",
            Self::Pcm => "PCM",
            Self::Flac => "FLAC",
            Self::DtsHdHra => "DTS-HD HRA",
            Self::Dts => "DTS",
            Self::DdPlus => "DD+",
            Self::Dd => "DD",
            Self::Opus => "OPUS",
            Self::Aac => "AAC",
            Self::Mp3 => "MP3",
            Self::Other => "AUDIO",
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::DtsX => 100,
            Self::TrueHdAtmos => 95,
            Self::DdPlusAtmos => 85,
            Self::DtsHdMa => 80,
            Self::TrueHd => 78,
            Self::Pcm => 75,
            Self::Flac => 74,
            Self::DtsHdHra => 60,
            Self::Dts => 50,
            Self::DdPlus => 45,
            Self::Dd => 40,
            Self::Opus => 30,
            Self::Aac => 25,
            Self::Mp3 => 10,
            Self::Other => 1,
        }
    }

    fn resolve(facts: &OverlayMediaFacts) -> Option<Self> {
        let codec = facts
            .audio_codec
            .as_deref()
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty());
        let profile = facts.audio_profile.as_deref().unwrap_or("").trim();
        let atmos = profile.contains("Atmos");
        if profile.contains("DTS:X") {
            return Some(Self::DtsX);
        }
        if profile.starts_with("DTS-HD MA") {
            return Some(Self::DtsHdMa);
        }
        if profile.starts_with("DTS-HD HRA") {
            return Some(Self::DtsHdHra);
        }
        let codec = codec?;
        Some(match codec.as_str() {
            "truehd" if atmos => Self::TrueHdAtmos,
            "truehd" | "mlp" => Self::TrueHd,
            "eac3" if atmos => Self::DdPlusAtmos,
            "eac3" => Self::DdPlus,
            "ac3" => Self::Dd,
            "dts" => Self::Dts,
            "flac" => Self::Flac,
            "opus" => Self::Opus,
            "aac" => Self::Aac,
            "mp3" => Self::Mp3,
            other if other.starts_with("pcm") || other == "lpcm" => Self::Pcm,
            _ => Self::Other,
        })
    }
}

/// A series' airing status. Metadata arrives TVDB-style (`Continuing`,
/// `Ended`, `Upcoming`); TMDB's statuses are folded onto the same four so a
/// template keeps working whichever source the metadata gateway uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlaySeriesStatus {
    Continuing,
    Upcoming,
    Ended,
    Canceled,
}

impl OverlaySeriesStatus {
    pub const ALL: [Self; 4] = [
        Self::Continuing,
        Self::Upcoming,
        Self::Ended,
        Self::Canceled,
    ];

    pub fn token(self) -> &'static str {
        match self {
            Self::Continuing => "continuing",
            Self::Upcoming => "upcoming",
            Self::Ended => "ended",
            Self::Canceled => "canceled",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Continuing => "CONTINUING",
            Self::Upcoming => "UPCOMING",
            Self::Ended => "ENDED",
            Self::Canceled => "CANCELED",
        }
    }

    pub fn from_token(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.token() == token)
    }

    /// The status for a title's stored `content_status`. Only series and
    /// anime have one: movies use some of the same words (`Planned`,
    /// `Canceled`) for their release state, which is not this field.
    pub fn resolve(facet: Option<&str>, content_status: Option<&str>) -> Option<Self> {
        if !matches!(facet, Some("series" | "anime")) {
            return None;
        }
        match content_status?.trim().to_ascii_lowercase().as_str() {
            "continuing" | "returning" | "returning series" | "airing" => Some(Self::Continuing),
            "upcoming" | "planned" | "pilot" | "in production" | "in development" => {
                Some(Self::Upcoming)
            }
            "ended" | "finished" => Some(Self::Ended),
            "canceled" | "cancelled" => Some(Self::Canceled),
            _ => None,
        }
    }
}

/// The resolved badge fields for one title.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OverlayFields {
    pub resolution: Option<OverlayResolution>,
    pub hdr: Option<OverlayHdr>,
    pub audio: Option<OverlayAudio>,
    pub audio_channels: Option<String>,
    pub edition: Option<String>,
    pub series_status: Option<OverlaySeriesStatus>,
}

impl OverlayFields {
    /// Best available value per field across every file of the title. For a
    /// series this is the best quality present in any episode.
    ///
    /// Edition belongs to a single cut, so it is taken only when every file
    /// that carries one agrees.
    pub fn aggregate(files: &[OverlayMediaFacts]) -> Self {
        let resolution = files.iter().filter_map(OverlayResolution::resolve).max();
        let hdr = files.iter().filter_map(OverlayHdr::resolve).max();
        let best_audio = files
            .iter()
            .filter_map(|facts| OverlayAudio::resolve(facts).map(|audio| (audio, facts)))
            .max_by(|(left, left_facts), (right, right_facts)| {
                left.rank()
                    .cmp(&right.rank())
                    .then(left_facts.audio_channels.cmp(&right_facts.audio_channels))
            });
        let audio = best_audio.map(|(audio, _)| audio);
        let audio_channels = best_audio
            .and_then(|(_, facts)| facts.audio_channels)
            .and_then(channel_layout);
        let mut editions = files
            .iter()
            .filter_map(|facts| facts.edition.as_deref())
            .map(str::trim)
            .filter(|edition| !edition.is_empty());
        let edition = match editions.next() {
            Some(first) if editions.all(|other| other.eq_ignore_ascii_case(first)) => {
                Some(first.to_string())
            }
            _ => None,
        };
        Self {
            resolution,
            hdr,
            audio,
            audio_channels,
            edition,
            series_status: None,
        }
    }

    /// Add the title-level fields that do not come from its files.
    pub fn with_title(mut self, facet: Option<&str>, content_status: Option<&str>) -> Self {
        self.series_status = OverlaySeriesStatus::resolve(facet, content_status);
        self
    }

    /// Placeholder values in a stable order. Absent fields map to the empty
    /// string so a template can test for presence.
    pub fn template_values(&self) -> BTreeMap<&'static str, String> {
        let mut values = BTreeMap::new();
        let token = |value: Option<&'static str>| value.unwrap_or("").to_string();
        values.insert("resolution", token(self.resolution.map(|v| v.token())));
        values.insert(
            "resolution_label",
            token(self.resolution.map(|v| v.label())),
        );
        values.insert("hdr", token(self.hdr.map(|v| v.token())));
        values.insert("hdr_label", token(self.hdr.map(|v| v.label())));
        values.insert("audio_codec", token(self.audio.map(|v| v.token())));
        values.insert("audio_label", token(self.audio.map(|v| v.label())));
        values.insert(
            "audio_channels",
            self.audio_channels.clone().unwrap_or_default(),
        );
        values.insert(
            "edition",
            self.edition
                .as_deref()
                .map(edition_token)
                .unwrap_or_default(),
        );
        values.insert(
            "edition_label",
            self.edition
                .as_deref()
                .map(|edition| edition.to_uppercase())
                .unwrap_or_default(),
        );
        values.insert(
            "series_status",
            token(self.series_status.map(|v| v.token())),
        );
        values.insert(
            "series_status_label",
            token(self.series_status.map(|v| v.label())),
        );
        values
    }

    /// Canonical `key=value` lines, the field component of `input_hash`.
    pub fn canonical(&self) -> String {
        let mut out = String::new();
        for (key, value) in self.template_values() {
            out.push_str(key);
            out.push('=');
            out.push_str(&value);
            out.push('\n');
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.resolution.is_none()
            && self.hdr.is_none()
            && self.audio.is_none()
            && self.edition.is_none()
            && self.series_status.is_none()
    }
}

/// Channel layouts `audio_channels` takes for common channel counts.
pub const CHANNEL_LAYOUTS: [&str; 6] = ["7.1", "6.1", "5.1", "2.1", "2.0", "1.0"];

/// Values a template condition may compare a field against, or `None` when
/// the field takes free-form values. A condition naming any other value can
/// never hold, so validation rejects it instead of saving a dead badge.
pub fn condition_values(field: &str) -> Option<Vec<&'static str>> {
    Some(match field {
        "resolution" => OverlayResolution::ALL
            .iter()
            .map(|value| value.token())
            .collect(),
        "resolution_label" => OverlayResolution::ALL
            .iter()
            .map(|value| value.label())
            .collect(),
        "hdr" => OverlayHdr::ALL.iter().map(|value| value.token()).collect(),
        "hdr_label" => OverlayHdr::ALL.iter().map(|value| value.label()).collect(),
        "audio_codec" => OverlayAudio::ALL
            .iter()
            .map(|value| value.token())
            .collect(),
        "audio_label" => OverlayAudio::ALL
            .iter()
            .map(|value| value.label())
            .collect(),
        "series_status" => OverlaySeriesStatus::ALL
            .iter()
            .map(|value| value.token())
            .collect(),
        "series_status_label" => OverlaySeriesStatus::ALL
            .iter()
            .map(|value| value.label())
            .collect(),
        _ => return None,
    })
}

/// Whether `value` is something `field` can ever resolve to. Closed fields
/// use `condition_values`; `audio_channels` also allows `Nch` for layouts
/// beyond 7.1, and `edition` is a lowercase slug as `edition_token` makes.
pub fn is_possible_condition_value(field: &str, value: &str) -> bool {
    if let Some(values) = condition_values(field) {
        return values.contains(&value);
    }
    match field {
        "audio_channels" => {
            CHANNEL_LAYOUTS.contains(&value)
                || value.strip_suffix("ch").is_some_and(|count| {
                    !count.is_empty() && count.bytes().all(|b| b.is_ascii_digit())
                })
        }
        "edition" => !value.is_empty() && edition_token(value) == value,
        _ => true,
    }
}

/// Longest edition a template preview accepts as a sample value.
pub const MAX_SAMPLE_EDITION_CHARS: usize = 80;

/// Sample values for previewing a template, as tokens. Empty or absent
/// fields are left unset, exactly as for a title without that data.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OverlaySampleValues {
    pub resolution: Option<String>,
    pub hdr: Option<String>,
    pub audio: Option<String>,
    pub audio_channels: Option<String>,
    pub edition: Option<String>,
    pub series_status: Option<String>,
}

impl OverlayFields {
    /// Fields for a preview. Unknown tokens are rejected rather than ignored
    /// so the preview never silently differs from what a title would show.
    pub fn from_sample(sample: &OverlaySampleValues) -> Result<Self, String> {
        fn present(value: &Option<String>) -> Option<&str> {
            value
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
        }
        fn parse<T>(
            value: &Option<String>,
            field: &str,
            from_token: fn(&str) -> Option<T>,
        ) -> Result<Option<T>, String> {
            present(value)
                .map(|token| {
                    from_token(token).ok_or_else(|| format!("unknown {field} \"{token}\""))
                })
                .transpose()
        }
        let audio_channels = present(&sample.audio_channels)
            .map(|layout| {
                CHANNEL_LAYOUTS
                    .contains(&layout)
                    .then(|| layout.to_string())
                    .ok_or_else(|| format!("unknown audio_channels \"{layout}\""))
            })
            .transpose()?;
        let edition = present(&sample.edition)
            .map(|edition| {
                if edition.chars().count() > MAX_SAMPLE_EDITION_CHARS {
                    Err(format!(
                        "edition must be at most {MAX_SAMPLE_EDITION_CHARS} characters"
                    ))
                } else {
                    Ok(edition.to_string())
                }
            })
            .transpose()?;
        Ok(Self {
            resolution: parse(
                &sample.resolution,
                "resolution",
                OverlayResolution::from_token,
            )?,
            hdr: parse(&sample.hdr, "hdr", OverlayHdr::from_token)?,
            audio: parse(&sample.audio, "audio_codec", OverlayAudio::from_token)?,
            audio_channels,
            edition,
            series_status: parse(
                &sample.series_status,
                "series_status",
                OverlaySeriesStatus::from_token,
            )?,
        })
    }
}

fn channel_layout(channels: i64) -> Option<String> {
    match channels {
        1 => Some("1.0".into()),
        2 => Some("2.0".into()),
        3 => Some("2.1".into()),
        6 => Some("5.1".into()),
        7 => Some("6.1".into()),
        8 => Some("7.1".into()),
        n if n > 8 => Some(format!("{n}ch")),
        _ => None,
    }
}

/// Lowercase ASCII slug for an edition, for conditional matching:
/// `Director's Cut` becomes `directors_cut`.
pub fn edition_token(edition: &str) -> String {
    let mut out = String::with_capacity(edition.len());
    let mut pending_separator = false;
    for ch in edition.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_separator && !out.is_empty() {
                out.push('_');
            }
            pending_separator = false;
            out.push(ch.to_ascii_lowercase());
        } else if ch == '\'' || ch == '\u{2019}' {
            continue;
        } else {
            pending_separator = true;
        }
    }
    out
}

/// Every placeholder and condition field a template may reference, in the
/// order `template_values` yields them. Part of the public contract.
pub const TEMPLATE_FIELDS: &[&str] = &[
    "audio_channels",
    "audio_codec",
    "audio_label",
    "edition",
    "edition_label",
    "hdr",
    "hdr_label",
    "resolution",
    "resolution_label",
    "series_status",
    "series_status_label",
];

/// Version of the template contract this build understands.
pub const TEMPLATE_SPEC_VERSION: u32 = 1;

/// Bumped when the renderer's output changes for identical inputs (encoder
/// settings, output sizes, font), so every poster rebuilds once.
pub const RENDERER_REVISION: u32 = 1;

pub fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// `template_version` for a template body: the contract version, renderer
/// revision and a content hash, so editing a template rebuilds its posters
/// without anyone having to bump a number.
pub fn template_version(svg: &str) -> String {
    format!(
        "{TEMPLATE_SPEC_VERSION}.{RENDERER_REVISION}:{}",
        blake3_hex(svg.as_bytes())
    )
}

/// `blake3(original_hash + template_version + resolved field values)`.
pub fn input_hash(original_hash: &str, template_version: &str, fields: &OverlayFields) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(original_hash.as_bytes());
    hasher.update(b"\0");
    hasher.update(template_version.as_bytes());
    hasher.update(b"\0");
    hasher.update(fields.canonical().as_bytes());
    hasher.finalize().to_hex().to_string()
}
