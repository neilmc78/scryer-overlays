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
fn edition_is_kept_only_when_every_file_agrees() {
    let cut = |edition: &str| OverlayMediaFacts {
        edition: Some(edition.into()),
        ..facts()
    };
    assert_eq!(
        OverlayFields::aggregate(&[cut("Director's Cut"), cut("director's cut"), facts()])
            .edition
            .as_deref(),
        Some("Director's Cut")
    );
    assert_eq!(
        OverlayFields::aggregate(&[cut("Director's Cut"), cut("IMAX")]).edition,
        None
    );
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
