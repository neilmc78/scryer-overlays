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
