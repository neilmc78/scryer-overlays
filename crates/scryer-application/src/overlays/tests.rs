use super::*;

fn facts() -> OverlayMediaFacts {
    OverlayMediaFacts::default()
}

fn uhd_dv_atmos() -> OverlayMediaFacts {
    OverlayMediaFacts {
        video_width: Some(3840),
        video_height: Some(1600),
        video_hdr_format: Some("Dolby Vision".into()),
        audio_codec: Some("truehd".into()),
        audio_profile: Some("Dolby TrueHD + Dolby Atmos".into()),
        audio_channels: Some(8),
        ..facts()
    }
}

fn hd_sdr_ac3() -> OverlayMediaFacts {
    OverlayMediaFacts {
        video_width: Some(1920),
        video_height: Some(1080),
        audio_codec: Some("ac3".into()),
        audio_channels: Some(6),
        ..facts()
    }
}

#[test]
fn input_hash_is_stable_for_identical_inputs() {
    let fields = OverlayFields::aggregate(&[uhd_dv_atmos()]);
    let version = template_version("<svg/>");
    assert_eq!(
        input_hash("abc", &version, &fields),
        input_hash("abc", &version, &fields.clone())
    );
}

#[test]
fn input_hash_changes_with_each_component() {
    let fields = OverlayFields::aggregate(&[uhd_dv_atmos()]);
    let other_fields = OverlayFields::aggregate(&[hd_sdr_ac3()]);
    let version = template_version("<svg/>");
    let base = input_hash("original-a", &version, &fields);

    assert_ne!(
        base,
        input_hash("original-b", &version, &fields),
        "new artwork"
    );
    assert_ne!(
        base,
        input_hash("original-a", &template_version("<svg></svg>"), &fields),
        "template edit"
    );
    assert_ne!(
        base,
        input_hash("original-a", &version, &other_fields),
        "upgrade"
    );
}

#[test]
fn input_hash_components_cannot_be_shifted_into_each_other() {
    let fields = OverlayFields::default();
    assert_ne!(
        input_hash("ab", "c", &fields),
        input_hash("a", "bc", &fields)
    );
}

#[test]
fn template_version_carries_spec_version_and_content_hash() {
    let version = template_version("<svg/>");
    assert!(version.starts_with(&format!("{TEMPLATE_SPEC_VERSION}.{RENDERER_REVISION}:")));
    assert!(version.ends_with(&blake3_hex(b"<svg/>")));
}

#[test]
fn resolution_prefers_probe_dimensions_and_handles_scope_crops() {
    let scope = OverlayMediaFacts {
        video_width: Some(3840),
        video_height: Some(1600),
        parsed_resolution: Some("1080p".into()),
        ..facts()
    };
    assert_eq!(
        OverlayFields::aggregate(&[scope]).resolution,
        Some(OverlayResolution::Uhd2160)
    );
    let unprobed = OverlayMediaFacts {
        parsed_resolution: Some("720p".into()),
        ..facts()
    };
    assert_eq!(
        OverlayFields::aggregate(&[unprobed]).resolution,
        Some(OverlayResolution::Hd720)
    );
}

#[test]
fn hdr_is_unknown_without_a_probe_and_sdr_with_one() {
    assert_eq!(OverlayFields::aggregate(&[facts()]).hdr, None);
    assert_eq!(
        OverlayFields::aggregate(&[hd_sdr_ac3()]).hdr,
        Some(OverlayHdr::Sdr)
    );
    let hdr10plus = OverlayMediaFacts {
        video_hdr_format: Some("HDR10+".into()),
        ..hd_sdr_ac3()
    };
    assert_eq!(
        OverlayFields::aggregate(&[hdr10plus]).hdr,
        Some(OverlayHdr::Hdr10Plus)
    );
}

#[test]
fn audio_profiles_map_to_contract_tokens() {
    let cases = [
        ("dts", "DTS-HD MA + DTS:X", "dtsx"),
        ("dts", "DTS-HD MA", "dtshd_ma"),
        ("truehd", "Dolby TrueHD + Dolby Atmos", "truehd_atmos"),
        ("eac3", "Dolby Digital Plus + Dolby Atmos", "ddp_atmos"),
        ("eac3", "", "ddp"),
        ("pcm_s24le", "", "pcm"),
        ("aac", "LC", "aac"),
        ("wmapro", "", "other"),
    ];
    for (codec, profile, expected) in cases {
        let file = OverlayMediaFacts {
            audio_codec: Some(codec.into()),
            audio_profile: (!profile.is_empty()).then(|| profile.into()),
            ..facts()
        };
        let fields = OverlayFields::aggregate(&[file]);
        assert_eq!(
            fields.audio.map(|audio| audio.token()),
            Some(expected),
            "{codec} / {profile}"
        );
    }
}

#[test]
fn series_aggregate_takes_the_best_value_per_field_across_episodes() {
    let fields = OverlayFields::aggregate(&[hd_sdr_ac3(), uhd_dv_atmos(), hd_sdr_ac3()]);
    assert_eq!(fields.resolution, Some(OverlayResolution::Uhd2160));
    assert_eq!(fields.hdr, Some(OverlayHdr::DolbyVision));
    assert_eq!(fields.audio, Some(OverlayAudio::TrueHdAtmos));
    assert_eq!(fields.audio_channels.as_deref(), Some("7.1"));
}

#[test]
fn every_distinct_edition_is_listed_primary_first() {
    let cut = |edition: &str, additional: bool| OverlayMediaFacts {
        edition: Some(edition.into()),
        additional,
        ..facts()
    };
    let fields = OverlayFields::aggregate(&[
        cut("Theatrical", true),
        cut("Director's Cut", false),
        cut("director's cut", false),
        facts(),
    ]);
    assert_eq!(fields.editions, vec!["Director's Cut", "Theatrical"]);
    let values = fields.template_values();
    assert_eq!(values["edition"], "directors_cut,theatrical");
    assert_eq!(values["edition_label"], "DIRECTOR'S CUT / THEATRICAL");
}

#[test]
fn edition_tags_in_file_names_fill_in_unparsed_editions() {
    let file = |path: &str| OverlayMediaFacts {
        file_path: Some(path.into()),
        ..facts()
    };
    for (path, expected) in [
        (
            "/mnt/media/Movies/Alien (1979)/Alien 1979 - {edition-Directors Cut}[Bluray-1080p].mkv",
            Some("Directors Cut"),
        ),
        (
            "/media/2001 A Space Odyssey 1968 - {Edition-Remastered}[Bluray-1080p].mkv",
            Some("Remastered"),
        ),
        ("/media/Movie {edition-}.mkv", None),
        ("/media/{edition-Cut}/Movie.mkv", None),
        ("/media/Movie (2001).mkv", None),
    ] {
        assert_eq!(file(path).edition_name().as_deref(), expected, "{path}");
    }
    // The parsed value wins over the file name.
    let parsed = OverlayMediaFacts {
        edition: Some("IMAX".into()),
        file_path: Some("/m/Movie {edition-Theatrical}.mkv".into()),
        ..facts()
    };
    assert_eq!(parsed.edition_name().as_deref(), Some("IMAX"));
    assert_eq!(
        edition_from_file_name("C:\\Movies\\Film {edition-Extended}.mkv").as_deref(),
        Some("Extended")
    );
}

#[test]
fn additional_versions_add_editions_but_not_quality() {
    let primary = OverlayMediaFacts {
        file_path: Some("/m/Alien 1979 - {edition-Directors Cut}[Bluray-1080p].mkv".into()),
        ..hd_sdr_ac3()
    };
    let additional = OverlayMediaFacts {
        file_path: Some("/m/Alien 1979 - {edition-Theatrical}[Remux-2160p].mkv".into()),
        additional: true,
        ..uhd_dv_atmos()
    };
    let fields = OverlayFields::aggregate(&[primary, additional.clone()]);
    assert_eq!(fields.resolution, Some(OverlayResolution::Hd1080));
    assert_eq!(fields.hdr, Some(OverlayHdr::Sdr));
    assert_eq!(fields.editions, vec!["Directors Cut", "Theatrical"]);
    // With no primary file, the additional versions supply quality too.
    let only_additional = OverlayFields::aggregate(&[additional]);
    assert_eq!(only_additional.resolution, Some(OverlayResolution::Uhd2160));
}

#[test]
fn edition_conditions_test_each_edition() {
    assert_eq!(
        condition_value_set("edition", "directors_cut,theatrical"),
        vec!["directors_cut", "theatrical"]
    );
    assert_eq!(
        condition_value_set("edition_label", "DIRECTOR'S CUT / THEATRICAL"),
        vec!["DIRECTOR'S CUT", "THEATRICAL"]
    );
    assert_eq!(
        condition_value_set("audio_label", "DD+ ATMOS"),
        vec!["DD+ ATMOS"]
    );
    assert!(condition_value_set("edition", "").is_empty());
}

#[test]
fn template_values_expose_every_contract_field() {
    let mut file = uhd_dv_atmos();
    file.edition = Some("Director's Cut".into());
    let values = OverlayFields::aggregate(&[file]).template_values();
    assert_eq!(values["resolution"], "2160p");
    assert_eq!(values["resolution_label"], "4K");
    assert_eq!(values["hdr"], "dv");
    assert_eq!(values["hdr_label"], "DOLBY VISION");
    assert_eq!(values["audio_codec"], "truehd_atmos");
    assert_eq!(values["audio_label"], "TRUEHD ATMOS");
    assert_eq!(values["audio_channels"], "7.1");
    assert_eq!(values["edition"], "directors_cut");
    assert_eq!(values["edition_label"], "DIRECTOR'S CUT");

    let empty = OverlayFields::default().template_values();
    assert!(empty.values().all(String::is_empty));
}

#[test]
fn edition_tokens_are_ascii_slugs() {
    assert_eq!(edition_token("Director's Cut"), "directors_cut");
    assert_eq!(edition_token("  Extended  Edition "), "extended_edition");
    assert_eq!(edition_token("IMAX Enhanced"), "imax_enhanced");
}

#[test]
fn route_variants_map_to_overlay_outputs() {
    assert_eq!(
        PosterOverlayVariant::from_route("original"),
        Some(PosterOverlayVariant::Full)
    );
    assert_eq!(
        PosterOverlayVariant::from_route("w250"),
        Some(PosterOverlayVariant::W250)
    );
    assert_eq!(PosterOverlayVariant::from_route("w1280"), None);
}

#[test]
fn presented_version_is_the_route_length_prefix() {
    assert_eq!(
        super::service::presented_version("0123456789abcdef0123"),
        "0123456789abcdef"
    );
    assert_eq!(super::service::presented_version("short"), "short");
}

#[test]
fn template_fields_list_matches_the_values_produced() {
    let keys = OverlayFields::default()
        .template_values()
        .into_keys()
        .collect::<Vec<_>>();
    assert_eq!(keys, TEMPLATE_FIELDS);
}

#[test]
fn sample_values_resolve_through_the_same_vocabulary_as_titles() {
    let sample = OverlaySampleValues {
        resolution: Some("2160p".into()),
        hdr: Some("dv".into()),
        audio: Some("truehd_atmos".into()),
        audio_channels: Some("7.1".into()),
        edition: Some("Director's Cut".into()),
        series_status: None,
        video_codec: None,
        source: None,
    };
    let from_sample = OverlayFields::from_sample(&sample).expect("valid sample");
    let from_title = OverlayFields::aggregate(&[OverlayMediaFacts {
        edition: Some("Director's Cut".into()),
        ..uhd_dv_atmos()
    }]);
    assert_eq!(from_sample.template_values(), from_title.template_values());
}

#[test]
fn blank_sample_values_leave_fields_unset() {
    let sample = OverlaySampleValues {
        resolution: Some("  ".into()),
        edition: Some(String::new()),
        ..OverlaySampleValues::default()
    };
    let fields = OverlayFields::from_sample(&sample).expect("blank is allowed");
    assert!(fields.is_empty());
}

#[test]
fn unknown_sample_tokens_and_oversized_editions_are_rejected() {
    for sample in [
        OverlaySampleValues {
            resolution: Some("8k".into()),
            ..OverlaySampleValues::default()
        },
        OverlaySampleValues {
            hdr: Some("DOLBY VISION".into()),
            ..OverlaySampleValues::default()
        },
        OverlaySampleValues {
            audio: Some("atmos".into()),
            ..OverlaySampleValues::default()
        },
        OverlaySampleValues {
            audio_channels: Some("9.2".into()),
            ..OverlaySampleValues::default()
        },
        OverlaySampleValues {
            edition: Some("x".repeat(MAX_SAMPLE_EDITION_CHARS + 1)),
            ..OverlaySampleValues::default()
        },
    ] {
        assert!(OverlayFields::from_sample(&sample).is_err(), "{sample:?}");
    }
}

#[test]
fn every_option_token_round_trips() {
    for value in OverlayResolution::ALL {
        assert_eq!(OverlayResolution::from_token(value.token()), Some(value));
    }
    for value in OverlayHdr::ALL {
        assert_eq!(OverlayHdr::from_token(value.token()), Some(value));
    }
    for value in OverlayAudio::ALL {
        assert_eq!(OverlayAudio::from_token(value.token()), Some(value));
    }
}

#[test]
fn series_status_maps_tvdb_and_tmdb_statuses_for_series_only() {
    for (facet, raw, expected) in [
        (
            "series",
            "Continuing",
            Some(OverlaySeriesStatus::Continuing),
        ),
        ("series", "Ended", Some(OverlaySeriesStatus::Ended)),
        ("anime", "Upcoming", Some(OverlaySeriesStatus::Upcoming)),
        (
            "series",
            "Returning Series",
            Some(OverlaySeriesStatus::Continuing),
        ),
        (
            "series",
            "In Production",
            Some(OverlaySeriesStatus::Upcoming),
        ),
        ("series", "Planned", Some(OverlaySeriesStatus::Upcoming)),
        ("series", "Pilot", Some(OverlaySeriesStatus::Upcoming)),
        ("series", "Canceled", Some(OverlaySeriesStatus::Canceled)),
        ("series", "cancelled", Some(OverlaySeriesStatus::Canceled)),
        ("series", "something new", None),
        // A movie's release state uses some of the same words; it is not a
        // series status.
        ("movie", "Canceled", None),
        ("movie", "Planned", None),
    ] {
        assert_eq!(
            OverlaySeriesStatus::resolve(Some(facet), Some(raw)),
            expected,
            "{facet} {raw}"
        );
    }
    assert_eq!(OverlaySeriesStatus::resolve(Some("series"), None), None);
}

#[test]
fn series_status_reaches_template_values_and_the_input_hash() {
    let continuing =
        OverlayFields::aggregate(&[hd_sdr_ac3()]).with_title(Some("series"), Some("Continuing"));
    let ended = OverlayFields::aggregate(&[hd_sdr_ac3()]).with_title(Some("series"), Some("Ended"));
    let values = ended.template_values();
    assert_eq!(values["series_status"], "ended");
    assert_eq!(values["series_status_label"], "ENDED");
    // A status change must change the hash, so the next reconcile re-renders.
    assert_ne!(
        input_hash("original", "v1", &continuing),
        input_hash("original", "v1", &ended)
    );
}

#[test]
fn only_title_poster_proxy_sources_carry_overlays() {
    let source = |owner_type: Option<&str>, owner_id: Option<&str>, kind: &str| {
        crate::ImageProxySourceRecord {
            token: "token".into(),
            upstream_url: Some("https://image.tmdb.org/t/p/w500/a.jpg".into()),
            owner_type: owner_type.map(str::to_string),
            owner_id: owner_id.map(str::to_string),
            image_kind: kind.into(),
            fallback_class: "portrait".into(),
            last_seen_at: chrono::Utc::now(),
        }
    };
    assert_eq!(
        overlay_title_for_proxy_source(&source(Some("title"), Some("t-1"), "poster")),
        Some("t-1")
    );
    for other in [
        source(Some("title"), Some("t-1"), "fanart"),
        source(Some("movie"), Some("m-1"), "poster"),
        source(Some("media_request"), Some("r-1"), "poster"),
        source(None, None, "poster"),
        source(Some("title"), Some(""), "poster"),
    ] {
        assert_eq!(overlay_title_for_proxy_source(&other), None, "{other:?}");
    }
}

#[test]
fn video_codec_comes_from_the_probe_then_the_release_name() {
    let codec = |probed: Option<&str>, parsed: Option<&str>| {
        OverlayFields::aggregate(&[OverlayMediaFacts {
            video_codec: probed.map(str::to_string),
            video_codec_parsed: parsed.map(str::to_string),
            ..facts()
        }])
        .video_codec
    };
    assert_eq!(
        codec(Some("hevc"), Some("x264")),
        Some(OverlayVideoCodec::H265)
    );
    assert_eq!(codec(Some("h264"), None), Some(OverlayVideoCodec::H264));
    assert_eq!(codec(Some("av1"), None), Some(OverlayVideoCodec::Av1));
    assert_eq!(
        codec(Some("mpeg2video"), None),
        Some(OverlayVideoCodec::Mpeg2)
    );
    assert_eq!(codec(None, Some("x265")), Some(OverlayVideoCodec::H265));
    assert_eq!(codec(None, Some("H.264")), Some(OverlayVideoCodec::H264));
    assert_eq!(codec(Some("prores"), None), None);
    assert_eq!(codec(None, None), None);

    let fields = OverlayFields::aggregate(&[OverlayMediaFacts {
        video_codec: Some("hevc".into()),
        ..facts()
    }]);
    let values = fields.template_values();
    assert_eq!(values["video_codec"], "h265");
    assert_eq!(values["video_codec_label"], "H.265");
}

#[test]
fn a_series_shows_its_most_modern_codec_and_additional_cuts_do_not_count() {
    let file = |codec: &str, additional: bool| OverlayMediaFacts {
        video_codec: Some(codec.into()),
        additional,
        ..facts()
    };
    assert_eq!(
        OverlayFields::aggregate(&[file("h264", false), file("hevc", false)]).video_codec,
        Some(OverlayVideoCodec::H265)
    );
    assert_eq!(
        OverlayFields::aggregate(&[file("h264", false), file("av1", true)]).video_codec,
        Some(OverlayVideoCodec::H264)
    );
    for value in OverlayVideoCodec::ALL {
        assert_eq!(OverlayVideoCodec::from_token(value.token()), Some(value));
    }
}

#[test]
fn source_is_the_best_release_source_of_the_primary_files() {
    let file = |source: &str, additional: bool| OverlayMediaFacts {
        source_type: Some(source.into()),
        additional,
        ..facts()
    };
    for (raw, expected) in [
        ("BluRay", OverlaySource::BluRay),
        ("Remux", OverlaySource::Remux),
        ("WEB-DL", OverlaySource::WebDl),
        ("WEBRip", OverlaySource::WebRip),
        ("DVD", OverlaySource::Dvd),
        ("HDTV", OverlaySource::Hdtv),
        ("BRDISK", OverlaySource::BrDisk),
    ] {
        assert_eq!(
            OverlayFields::aggregate(&[file(raw, false)]).source,
            Some(expected),
            "{raw}"
        );
    }
    assert_eq!(
        OverlayFields::aggregate(&[file("WEB-DL", false), file("BluRay", false)]).source,
        Some(OverlaySource::BluRay)
    );
    assert_eq!(
        OverlayFields::aggregate(&[file("DVD", false), file("Remux", true)]).source,
        Some(OverlaySource::Dvd)
    );
    assert_eq!(
        OverlayFields::aggregate(&[file("nonsense", false)]).source,
        None
    );
    let values = OverlayFields::aggregate(&[file("BluRay", false)]).template_values();
    assert_eq!(values["source"], "bluray");
    assert_eq!(values["source_label"], "BLURAY");
    for value in OverlaySource::ALL {
        assert_eq!(OverlaySource::from_token(value.token()), Some(value));
    }
}

fn rating(source: &str, value: Option<f64>, score: Option<f64>) -> crate::TitleExternalRating {
    crate::TitleExternalRating {
        source: source.to_string(),
        value,
        score,
        normalized: 0.0,
        votes: None,
        url: String::new(),
    }
}

#[test]
fn ratings_are_written_as_the_web_ui_shows_them() {
    let fields = OverlayFields::default().with_ratings(&[
        rating("imdb", Some(7.8), None),
        rating("Rotten Tomatoes", Some(74.0), None),
        rating("audience", None, Some(0.86)),
        rating("metacritic", Some(81.0), None),
        rating("mc-user", Some(7.9), None),
        rating("letterboxd", Some(4.0), None),
        rating("TMDb", Some(7.25), None),
        rating("trakt", Some(80.0), None),
    ]);
    let values = fields.template_values();
    assert_eq!(values["rating_imdb"], "7.8");
    assert_eq!(values["rating_rottentomatoes"], "74%");
    assert_eq!(values["rating_popcornmeter"], "86%");
    assert_eq!(values["rating_metacritic"], "81");
    assert_eq!(values["rating_metacritic_user"], "79");
    assert_eq!(values["rating_letterboxd"], "4");
    assert_eq!(values["rating_tmdb"], "7.3");
    assert_eq!(values["rating_trakt"], "80");
    assert_eq!(values["rating_mdblist"], "", "absent sources are empty");
}

#[test]
fn a_preferred_alias_wins_and_ratings_feed_the_input_hash() {
    let fields = OverlayFields::default().with_ratings(&[
        rating("audience", Some(60.0), None),
        rating("popcornmeter", Some(90.0), None),
    ]);
    assert_eq!(
        fields
            .ratings
            .get(&OverlayRatingSource::Popcornmeter)
            .map(String::as_str),
        Some("90%")
    );
    let before = OverlayFields::default().with_ratings(&[rating("imdb", Some(7.8), None)]);
    let after = OverlayFields::default().with_ratings(&[rating("imdb", Some(7.9), None)]);
    assert_ne!(
        input_hash("o", "t", &before),
        input_hash("o", "t", &after),
        "a changed score rebuilds the poster"
    );
    for source in OverlayRatingSource::ALL {
        assert_eq!(
            OverlayRatingSource::from_token(source.token()),
            Some(source)
        );
        assert!(
            TEMPLATE_FIELDS.contains(&source.field()),
            "{}",
            source.field()
        );
    }
}

#[test]
fn plex_maintenance_hours_hold_work_back_until_they_end() {
    use chrono::NaiveTime;
    let at = |hour, minute| NaiveTime::from_hms_opt(hour, minute, 0).unwrap();
    let window = |start_hour, end_hour| PlexMaintenanceWindow {
        start_hour,
        end_hour,
    };
    let minutes = |duration: Option<std::time::Duration>| duration.map(|d| d.as_secs() / 60);

    assert_eq!(
        minutes(maintenance_remaining(window(3, 5), at(3, 0))),
        Some(120)
    );
    assert_eq!(
        minutes(maintenance_remaining(window(3, 5), at(4, 30))),
        Some(30)
    );
    assert_eq!(maintenance_remaining(window(3, 5), at(5, 0)), None);
    assert_eq!(maintenance_remaining(window(3, 5), at(2, 59)), None);
    // A window past midnight.
    assert_eq!(
        minutes(maintenance_remaining(window(23, 2), at(23, 30))),
        Some(150)
    );
    assert_eq!(
        minutes(maintenance_remaining(window(23, 2), at(1, 0))),
        Some(60)
    );
    assert_eq!(maintenance_remaining(window(23, 2), at(12, 0)), None);
    // Equal hours mean no window.
    assert_eq!(maintenance_remaining(window(4, 4), at(4, 0)), None);
}
