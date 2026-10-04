//! Badge fields: normalisation of probe data into the closed vocabulary the
//! template contract exposes, aggregation across a title's files, and the
//! hashes that decide whether a poster needs rebuilding.
//!
//! Everything here is pure policy. The vocabulary is part of the public
//! template contract (`docs/poster-overlays.md`): a token may be added, but an
//! existing token must never change meaning.

use std::collections::BTreeMap;

use crate::TitleExternalRating;

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
    /// Probed codec name (`hevc`, `h264`, ...).
    pub video_codec: Option<String>,
    /// Release-name codec (`H.265`, `x264`, ...), used when not probed.
    pub video_codec_parsed: Option<String>,
    /// Release source from the file's release name (`BluRay`, `WEB-DL`,
    /// `Remux`, ...).
    pub source_type: Option<String>,
    /// The file's path; its `{edition-...}` tag names the edition when the
    /// parsed `edition` is empty.
    pub file_path: Option<String>,
    /// An additional version beside the primary file (another cut in the
    /// same folder). It contributes its edition, not its quality.
    pub additional: bool,
}

impl OverlayMediaFacts {
    /// This file's edition: the parsed value, else the Plex/Radarr-style
    /// `{edition-Name}` tag in the file name.
    pub fn edition_name(&self) -> Option<String> {
        self.edition
            .as_deref()
            .map(str::trim)
            .filter(|edition| !edition.is_empty())
            .map(str::to_string)
            .or_else(|| self.file_path.as_deref().and_then(edition_from_file_name))
    }

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

/// Video codec. Ordered oldest to newest, so `max` picks the most modern.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum OverlayVideoCodec {
    Mpeg2,
    Divx,
    Xvid,
    Mpeg4,
    Vc1,
    H264,
    Vp9,
    H265,
    Av1,
    Vvc,
}

impl OverlayVideoCodec {
    /// Newest first: the options a template preview offers.
    pub const ALL: [Self; 10] = [
        Self::Vvc,
        Self::Av1,
        Self::H265,
        Self::Vp9,
        Self::H264,
        Self::Vc1,
        Self::Mpeg4,
        Self::Xvid,
        Self::Divx,
        Self::Mpeg2,
    ];

    pub fn token(self) -> &'static str {
        match self {
            Self::Vvc => "h266",
            Self::Av1 => "av1",
            Self::H265 => "h265",
            Self::Vp9 => "vp9",
            Self::H264 => "h264",
            Self::Vc1 => "vc1",
            Self::Mpeg4 => "mpeg4",
            Self::Xvid => "xvid",
            Self::Divx => "divx",
            Self::Mpeg2 => "mpeg2",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Vvc => "H.266",
            Self::Av1 => "AV1",
            Self::H265 => "H.265",
            Self::Vp9 => "VP9",
            Self::H264 => "H.264",
            Self::Vc1 => "VC-1",
            Self::Mpeg4 => "MPEG-4",
            Self::Xvid => "XVID",
            Self::Divx => "DIVX",
            Self::Mpeg2 => "MPEG-2",
        }
    }

    pub fn from_token(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.token() == token)
    }

    /// A probe or release-name codec name, through the release parser's
    /// vocabulary (`hevc`, `x265` and `H.265` are all H.265). ffprobe's
    /// `mpeg2video` style names lose their `video` suffix first.
    fn parse(raw: &str) -> Option<Self> {
        use crate::VideoCodec;
        let raw = raw.trim();
        let parsed = VideoCodec::parse(raw).or_else(|| {
            raw.to_ascii_lowercase()
                .strip_suffix("video")
                .and_then(VideoCodec::parse)
        })?;
        Some(match parsed {
            VideoCodec::H264 => Self::H264,
            VideoCodec::H265 => Self::H265,
            VideoCodec::Av1 => Self::Av1,
            VideoCodec::Vp9 => Self::Vp9,
            VideoCodec::Vc1 => Self::Vc1,
            VideoCodec::Mpeg2 => Self::Mpeg2,
            VideoCodec::Mpeg4 => Self::Mpeg4,
            VideoCodec::Xvid => Self::Xvid,
            VideoCodec::Divx => Self::Divx,
            VideoCodec::Vvc => Self::Vvc,
        })
    }

    fn resolve(facts: &OverlayMediaFacts) -> Option<Self> {
        facts
            .video_codec
            .as_deref()
            .and_then(Self::parse)
            .or_else(|| facts.video_codec_parsed.as_deref().and_then(Self::parse))
    }
}

/// Release source. Ordered worst to best, so `max` picks the best copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum OverlaySource {
    Workprint,
    Cam,
    Telesync,
    Telecine,
    DvdScr,
    Dvd,
    Hdtv,
    WebRip,
    WebDl,
    BluRay,
    BrDisk,
    Remux,
}

impl OverlaySource {
    /// Best first: the options a template preview offers.
    pub const ALL: [Self; 12] = [
        Self::Remux,
        Self::BrDisk,
        Self::BluRay,
        Self::WebDl,
        Self::WebRip,
        Self::Hdtv,
        Self::Dvd,
        Self::DvdScr,
        Self::Telecine,
        Self::Telesync,
        Self::Cam,
        Self::Workprint,
    ];

    pub fn token(self) -> &'static str {
        match self {
            Self::Remux => "remux",
            Self::BrDisk => "brdisk",
            Self::BluRay => "bluray",
            Self::WebDl => "webdl",
            Self::WebRip => "webrip",
            Self::Hdtv => "hdtv",
            Self::Dvd => "dvd",
            Self::DvdScr => "dvdscr",
            Self::Telecine => "telecine",
            Self::Telesync => "telesync",
            Self::Cam => "cam",
            Self::Workprint => "workprint",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Remux => "REMUX",
            Self::BrDisk => "BR-DISK",
            Self::BluRay => "BLURAY",
            Self::WebDl => "WEB-DL",
            Self::WebRip => "WEBRIP",
            Self::Hdtv => "HDTV",
            Self::Dvd => "DVD",
            Self::DvdScr => "DVDSCR",
            Self::Telecine => "TELECINE",
            Self::Telesync => "TELESYNC",
            Self::Cam => "CAM",
            Self::Workprint => "WORKPRINT",
        }
    }

    pub fn from_token(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.token() == token)
    }

    /// A stored `source_type`: `Remux`, or a release parser source name.
    fn parse(raw: &str) -> Option<Self> {
        use crate::ReleaseSource;
        let raw = raw.trim();
        if raw.eq_ignore_ascii_case("remux") {
            return Some(Self::Remux);
        }
        Some(match ReleaseSource::parse(raw)? {
            ReleaseSource::WebDl => Self::WebDl,
            ReleaseSource::WebRip => Self::WebRip,
            ReleaseSource::BluRay => Self::BluRay,
            ReleaseSource::BrDisk => Self::BrDisk,
            ReleaseSource::Dvd => Self::Dvd,
            ReleaseSource::Hdtv => Self::Hdtv,
            ReleaseSource::Cam => Self::Cam,
            ReleaseSource::Telesync => Self::Telesync,
            ReleaseSource::Telecine => Self::Telecine,
            ReleaseSource::DvdScr => Self::DvdScr,
            ReleaseSource::Workprint => Self::Workprint,
        })
    }

    fn resolve(facts: &OverlayMediaFacts) -> Option<Self> {
        facts.source_type.as_deref().and_then(Self::parse)
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

/// A rating source a badge can show, as metadata stores it on the title.
/// The order is the display order the web UI uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum OverlayRatingSource {
    Imdb,
    RottenTomatoes,
    Popcornmeter,
    Metacritic,
    MetacriticUser,
    Letterboxd,
    Tmdb,
    Trakt,
    Mdblist,
}

/// How a rating source's score is written, matching the web UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RatingFormat {
    /// The source's own value, such as IMDb's `7.8`.
    Value,
    /// A percentage, such as Rotten Tomatoes' `74%`.
    Percent,
    /// A score out of 100, such as Metacritic's `81`.
    Hundred,
}

impl OverlayRatingSource {
    pub const ALL: [Self; 9] = [
        Self::Imdb,
        Self::RottenTomatoes,
        Self::Popcornmeter,
        Self::Metacritic,
        Self::MetacriticUser,
        Self::Letterboxd,
        Self::Tmdb,
        Self::Trakt,
        Self::Mdblist,
    ];

    pub fn token(self) -> &'static str {
        match self {
            Self::Imdb => "imdb",
            Self::RottenTomatoes => "rottentomatoes",
            Self::Popcornmeter => "popcornmeter",
            Self::Metacritic => "metacritic",
            Self::MetacriticUser => "metacritic_user",
            Self::Letterboxd => "letterboxd",
            Self::Tmdb => "tmdb",
            Self::Trakt => "trakt",
            Self::Mdblist => "mdblist",
        }
    }

    pub fn from_token(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|source| source.token() == token)
    }

    /// The template field holding this source's displayed score.
    pub fn field(self) -> &'static str {
        match self {
            Self::Imdb => "rating_imdb",
            Self::RottenTomatoes => "rating_rottentomatoes",
            Self::Popcornmeter => "rating_popcornmeter",
            Self::Metacritic => "rating_metacritic",
            Self::MetacriticUser => "rating_metacritic_user",
            Self::Letterboxd => "rating_letterboxd",
            Self::Tmdb => "rating_tmdb",
            Self::Trakt => "rating_trakt",
            Self::Mdblist => "rating_mdblist",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Imdb => "IMDb",
            Self::RottenTomatoes => "Rotten Tomatoes",
            Self::Popcornmeter => "Popcornmeter",
            Self::Metacritic => "Metacritic",
            Self::MetacriticUser => "Metacritic User",
            Self::Letterboxd => "Letterboxd",
            Self::Tmdb => "TMDB",
            Self::Trakt => "Trakt",
            Self::Mdblist => "MDBList",
        }
    }

    /// The bundled logo drawn for this source. Metacritic's user score
    /// shares the Metacritic logo, as it does in the web UI.
    pub fn logo(self) -> &'static str {
        match self {
            Self::MetacriticUser => "metacritic",
            other => other.token(),
        }
    }

    /// Stored source names for this rating, most preferred first, after
    /// the normalisation `normalized_rating_source` applies.
    fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::Imdb => &["imdb"],
            Self::RottenTomatoes => &["rottentomatoes", "tomatoes"],
            Self::Popcornmeter => &["popcornmeter", "popcorn", "audience"],
            Self::Metacritic => &["metacritic"],
            Self::MetacriticUser => &["metacriticuser", "mcuser"],
            Self::Letterboxd => &["letterboxd"],
            Self::Tmdb => &["tmdb"],
            Self::Trakt => &["trakt"],
            Self::Mdblist => &["mdblist"],
        }
    }

    fn format(self) -> RatingFormat {
        match self {
            Self::RottenTomatoes | Self::Popcornmeter => RatingFormat::Percent,
            Self::Metacritic | Self::MetacriticUser => RatingFormat::Hundred,
            _ => RatingFormat::Value,
        }
    }

    /// The displayed score for this source from a title's stored ratings,
    /// or `None` when the title has none from it.
    fn display(self, ratings: &[TitleExternalRating]) -> Option<String> {
        let rating = self.aliases().iter().find_map(|alias| {
            ratings
                .iter()
                .find(|rating| normalized_rating_source(&rating.source) == *alias)
        })?;
        let text = match self.format() {
            RatingFormat::Percent => format!("{}%", score_out_of_hundred(rating).round()),
            RatingFormat::Hundred => compact_rating(score_out_of_hundred(rating)),
            RatingFormat::Value => compact_rating(rating.value.unwrap_or(rating.normalized)),
        };
        Some(text)
    }
}

/// Lowercase with spaces, `_`, `.` and `-` removed: `Rotten Tomatoes` and
/// `rotten_tomatoes` are both `rottentomatoes`.
fn normalized_rating_source(source: &str) -> String {
    source
        .trim()
        .chars()
        .filter(|ch| !ch.is_whitespace() && !matches!(ch, '_' | '.' | '-'))
        .flat_map(char::to_lowercase)
        .collect()
}

fn score_out_of_hundred(rating: &TitleExternalRating) -> f64 {
    match rating.score.or(rating.value) {
        Some(score) if score <= 1.0 => score * 100.0,
        Some(score) if score <= 10.0 => score * 10.0,
        Some(score) => score,
        None => rating.normalized * 10.0,
    }
}

/// One decimal place at most, without a trailing `.0`: `7.8`, `8`, `74`.
fn compact_rating(value: f64) -> String {
    // Half away from zero, as the web UI's `toFixed` does for these values.
    let rounded = format!("{:.1}", (value * 10.0).round() / 10.0);
    rounded
        .strip_suffix(".0")
        .map(str::to_string)
        .unwrap_or(rounded)
}

/// The resolved badge fields for one title.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OverlayFields {
    pub resolution: Option<OverlayResolution>,
    pub hdr: Option<OverlayHdr>,
    pub audio: Option<OverlayAudio>,
    pub audio_channels: Option<String>,
    /// Every distinct edition present, primary first.
    pub editions: Vec<String>,
    pub series_status: Option<OverlaySeriesStatus>,
    pub video_codec: Option<OverlayVideoCodec>,
    pub source: Option<OverlaySource>,
    /// Displayed score per rating source the title has.
    pub ratings: BTreeMap<OverlayRatingSource, String>,
}

impl OverlayFields {
    /// Best available value per field across the title's primary files. For
    /// a series this is the best quality present in any episode.
    ///
    /// Editions come from every file, additional versions included, so a
    /// folder holding a theatrical cut and a director's cut lists both.
    pub fn aggregate(files: &[OverlayMediaFacts]) -> Self {
        let primary = files
            .iter()
            .filter(|facts| !facts.additional)
            .collect::<Vec<_>>();
        // A title whose only files are additional still gets quality badges.
        let quality = if primary.is_empty() {
            files.iter().collect::<Vec<_>>()
        } else {
            primary
        };
        let resolution = quality
            .iter()
            .copied()
            .filter_map(OverlayResolution::resolve)
            .max();
        let hdr = quality
            .iter()
            .copied()
            .filter_map(OverlayHdr::resolve)
            .max();
        let video_codec = quality
            .iter()
            .copied()
            .filter_map(OverlayVideoCodec::resolve)
            .max();
        let source = quality
            .iter()
            .copied()
            .filter_map(OverlaySource::resolve)
            .max();
        let best_audio = quality
            .iter()
            .copied()
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
        // Primary files first, then additional versions; one entry per
        // distinct edition.
        let mut editions: Vec<String> = Vec::new();
        for facts in files
            .iter()
            .filter(|facts| !facts.additional)
            .chain(files.iter().filter(|facts| facts.additional))
        {
            if let Some(name) = facts.edition_name()
                && !edition_token(&name).is_empty()
                && !editions
                    .iter()
                    .any(|known| edition_token(known) == edition_token(&name))
            {
                editions.push(name);
            }
        }
        Self {
            resolution,
            hdr,
            audio,
            audio_channels,
            editions,
            series_status: None,
            video_codec,
            source,
            ratings: BTreeMap::new(),
        }
    }

    /// Add the title-level fields that do not come from its files.
    pub fn with_title(mut self, facet: Option<&str>, content_status: Option<&str>) -> Self {
        self.series_status = OverlaySeriesStatus::resolve(facet, content_status);
        self
    }

    /// Add the title's ratings, formatted as the web UI shows them.
    pub fn with_ratings(mut self, ratings: &[TitleExternalRating]) -> Self {
        self.ratings = OverlayRatingSource::ALL
            .into_iter()
            .filter_map(|source| source.display(ratings).map(|text| (source, text)))
            .collect();
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
            self.editions
                .iter()
                .map(|edition| edition_token(edition))
                .collect::<Vec<_>>()
                .join(EDITION_TOKEN_SEPARATOR),
        );
        values.insert(
            "edition_label",
            self.editions
                .iter()
                .map(|edition| edition.to_uppercase())
                .collect::<Vec<_>>()
                .join(EDITION_LABEL_SEPARATOR),
        );
        values.insert(
            "series_status",
            token(self.series_status.map(|v| v.token())),
        );
        values.insert(
            "series_status_label",
            token(self.series_status.map(|v| v.label())),
        );
        values.insert("source", token(self.source.map(|v| v.token())));
        values.insert("source_label", token(self.source.map(|v| v.label())));
        values.insert("video_codec", token(self.video_codec.map(|v| v.token())));
        values.insert(
            "video_codec_label",
            token(self.video_codec.map(|v| v.label())),
        );
        for source in OverlayRatingSource::ALL {
            values.insert(
                source.field(),
                self.ratings.get(&source).cloned().unwrap_or_default(),
            );
        }
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
            && self.editions.is_empty()
            && self.series_status.is_none()
            && self.video_codec.is_none()
            && self.source.is_none()
            && self.ratings.is_empty()
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
        "source" => OverlaySource::ALL
            .iter()
            .map(|value| value.token())
            .collect(),
        "source_label" => OverlaySource::ALL
            .iter()
            .map(|value| value.label())
            .collect(),
        "video_codec" => OverlayVideoCodec::ALL
            .iter()
            .map(|value| value.token())
            .collect(),
        "video_codec_label" => OverlayVideoCodec::ALL
            .iter()
            .map(|value| value.label())
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
    pub video_codec: Option<String>,
    pub source: Option<String>,
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
            editions: edition
                .map(|edition| {
                    edition
                        .split(',')
                        .map(str::trim)
                        .filter(|name| !edition_token(name).is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            series_status: parse(
                &sample.series_status,
                "series_status",
                OverlaySeriesStatus::from_token,
            )?,
            video_codec: parse(
                &sample.video_codec,
                "video_codec",
                OverlayVideoCodec::from_token,
            )?,
            source: parse(&sample.source, "source", OverlaySource::from_token)?,
            ratings: BTreeMap::new(),
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

/// `edition` holds every edition's token, comma separated; a condition
/// matches when any of them equals a listed value.
pub const EDITION_TOKEN_SEPARATOR: &str = ",";
/// `edition_label` joins the display names.
pub const EDITION_LABEL_SEPARATOR: &str = " / ";

/// The values a condition compares against for one field: each edition
/// separately for the multi-valued edition fields, the whole value
/// otherwise.
pub fn condition_value_set<'v>(field: &str, value: &'v str) -> Vec<&'v str> {
    if value.is_empty() {
        return Vec::new();
    }
    match field {
        "edition" => value.split(EDITION_TOKEN_SEPARATOR).collect(),
        "edition_label" => value.split(EDITION_LABEL_SEPARATOR).collect(),
        _ => vec![value],
    }
}

/// The name in a `{edition-Name}` tag (Plex and Radarr naming) in the file
/// name of `path`.
pub fn edition_from_file_name(path: &str) -> Option<String> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let lower = name.to_ascii_lowercase();
    let start = lower.find("{edition-")? + "{edition-".len();
    let end = start + lower[start..].find('}')?;
    let edition = name[start..end].trim();
    (!edition.is_empty()).then(|| edition.to_string())
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
    "rating_imdb",
    "rating_letterboxd",
    "rating_mdblist",
    "rating_metacritic",
    "rating_metacritic_user",
    "rating_popcornmeter",
    "rating_rottentomatoes",
    "rating_tmdb",
    "rating_trakt",
    "resolution",
    "resolution_label",
    "series_status",
    "series_status_label",
    "source",
    "source_label",
    "video_codec",
    "video_codec_label",
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
