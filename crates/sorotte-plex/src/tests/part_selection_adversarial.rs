use super::*;

const DESIRED_SELECTION_INVARIANT: &str =
    "TC-PLEX-001: Plex part selection must use filename and size evidence";

fn playable_part(
    id: &str,
    file_name: Option<&str>,
    duration_millis: Option<u64>,
    size_bytes: Option<u64>,
) -> PlexPlayablePart {
    PlexPlayablePart {
        id: id.to_owned(),
        key: format!("/library/parts/{id}/file.mkv"),
        file_name: file_name.map(ToOwned::to_owned),
        duration_millis,
        size_bytes,
        container: Some("mkv".to_owned()),
    }
}

fn metadata(
    rating_key: &str,
    duration_millis: Option<u64>,
    parts: Vec<PlexPlayablePart>,
) -> PlexMediaMetadata {
    PlexMediaMetadata {
        rating_key: rating_key.to_owned(),
        title: format!("Fixture {rating_key}"),
        media_type: PlexMediaType::Movie,
        duration_millis,
        parts,
    }
}

fn oracle_basename(path: &str) -> Option<&str> {
    path.rsplit(['/', '\\'])
        .next()
        .map(str::trim)
        .filter(|name| !name.is_empty())
}

fn oracle_ascii_fold(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn reference_choose_part(
    parts: &[PlexPlayablePart],
    file_name: Option<&str>,
    size_bytes: Option<u64>,
    duration_millis: Option<u64>,
) -> Result<String, Vec<String>> {
    let mut candidates = parts.iter().collect::<Vec<_>>();

    if let Some(file_name) = file_name.and_then(oracle_basename) {
        let exact = candidates
            .iter()
            .copied()
            .filter(|part| {
                part.file_name
                    .as_deref()
                    .and_then(oracle_basename)
                    .is_some_and(|name| name == file_name)
            })
            .collect::<Vec<_>>();
        if !exact.is_empty() {
            candidates = exact;
        } else {
            let normalized_file_name = oracle_ascii_fold(file_name);
            let folded = candidates
                .iter()
                .copied()
                .filter(|part| {
                    part.file_name
                        .as_deref()
                        .and_then(oracle_basename)
                        .map(oracle_ascii_fold)
                        .is_some_and(|name| name == normalized_file_name)
                })
                .collect::<Vec<_>>();
            if !folded.is_empty() {
                candidates = folded;
            }
        }
    }

    if let Some(size_bytes) = size_bytes {
        let exact = candidates
            .iter()
            .copied()
            .filter(|part| part.size_bytes == Some(size_bytes))
            .collect::<Vec<_>>();
        if !exact.is_empty() {
            candidates = exact;
        }
    }

    if let Some(duration_millis) = duration_millis {
        let best_difference = candidates
            .iter()
            .filter_map(|part| {
                part.duration_millis
                    .map(|part_duration| part_duration.abs_diff(duration_millis))
            })
            .min();
        if let Some(best_difference) = best_difference {
            candidates.retain(|part| {
                part.duration_millis.is_some_and(|part_duration| {
                    part_duration.abs_diff(duration_millis) == best_difference
                })
            });
        }
    }

    if candidates.len() == 1 {
        Ok(candidates[0].key.clone())
    } else {
        Err(candidates.iter().map(|part| part.key.clone()).collect())
    }
}

fn playlist_target(
    rating_key: &str,
    file_name: Option<&str>,
    duration_millis: Option<u64>,
    size_bytes: Option<u64>,
) -> String {
    format_plex_playlist_uri(&PlexPlaylistUri {
        machine_identifier: "abc123machine".to_owned(),
        rating_key: rating_key.to_owned(),
        title: Some(format!("Fixture {rating_key}")),
        file_name: file_name.map(ToOwned::to_owned),
        duration_millis,
        size_bytes,
        media_type: Some(PlexMediaType::Movie),
    })
}

fn observed_part_for_playlist_target(
    metadata: PlexMediaMetadata,
    target: &str,
) -> Result<String, String> {
    let rating_key = metadata.rating_key.clone();
    let transport = FakeTransport::default();
    transport
        .metadata_results
        .borrow_mut()
        .insert(rating_key, metadata);
    let mut resolver = PlexMediaResolver::new(
        stream_resolver_config(),
        transport.clone(),
        PlexMatchCache::default(),
    );

    match resolver.resolve_stream_target(target, SystemTime::UNIX_EPOCH) {
        Ok(Some(_)) => transport
            .stream_parts
            .borrow()
            .last()
            .cloned()
            .ok_or_else(|| "resolver returned a target without selecting a part".to_owned()),
        Ok(None) => Err("resolver returned no stream target".to_owned()),
        Err(error) => Err(error.to_string()),
    }
}

fn observed_part_for_plain_shared_filename(
    metadata: PlexMediaMetadata,
    target: &str,
) -> Result<String, String> {
    let rating_key = metadata.rating_key.clone();
    let transport = FakeTransport::default();
    transport
        .file_search_results
        .borrow_mut()
        .push(PlexMediaSearchResult {
            rating_key: rating_key.clone(),
            title: metadata.title.clone(),
            parent_title: None,
            grandparent_title: None,
            media_type: PlexMediaType::Movie,
            duration_millis: metadata.duration_millis,
            file_paths: vec![format!("/library/movies/{target}")],
        });
    transport
        .metadata_results
        .borrow_mut()
        .insert(rating_key, metadata);
    let mut resolver = PlexMediaResolver::new(
        stream_resolver_config(),
        transport.clone(),
        PlexMatchCache::default(),
    );

    match resolver.resolve_stream_target(target, SystemTime::UNIX_EPOCH) {
        Ok(Some(_)) => transport
            .stream_parts
            .borrow()
            .last()
            .cloned()
            .ok_or_else(|| "resolver returned a target without selecting a part".to_owned()),
        Ok(None) => Err("resolver returned no stream target".to_owned()),
        Err(error) => Err(error.to_string()),
    }
}

fn record_mismatch(
    mismatches: &mut Vec<String>,
    case: &str,
    expected_part_id: &str,
    observed: Result<String, String>,
) {
    let expected = format!("/library/parts/{expected_part_id}/file.mkv");
    match observed {
        Ok(selected) if selected == expected => {}
        Ok(selected) => {
            mismatches.push(format!("{case}: expected {expected}, selected {selected}"))
        }
        Err(error) => mismatches.push(format!("{case}: expected {expected}, got error {error}")),
    }
}

#[test]
#[should_panic(expected = "TC-PLEX-001: Plex part selection must use filename and size evidence")]
fn known_defect_resolver_ignores_filename_and_size_evidence() {
    let mut mismatches = Vec::new();

    let plain_parts = vec![
        playable_part(
            "plain-exact",
            Some("Shared.Movie.1080p.mkv"),
            Some(7_200_000),
            Some(4_000_000_000),
        ),
        playable_part(
            "plain-other-version",
            Some("Shared.Movie.2160p.mkv"),
            Some(7_200_000),
            Some(12_000_000_000),
        ),
    ];
    for (order, parts) in [
        ("forward", plain_parts.clone()),
        ("reverse", plain_parts.into_iter().rev().collect()),
    ] {
        assert_eq!(
            reference_choose_part(
                &parts,
                Some("Shared.Movie.1080p.mkv"),
                None,
                Some(7_200_000),
            ),
            Ok("/library/parts/plain-exact/file.mkv".to_owned()),
            "the independent evidence oracle must identify the plain shared filename"
        );
        record_mismatch(
            &mut mismatches,
            &format!("plain-shared-filename-{order}"),
            "plain-exact",
            observed_part_for_plain_shared_filename(
                metadata("plain-shared", Some(7_200_000), parts),
                "Shared.Movie.1080p.mkv",
            ),
        );
    }

    struct Case {
        name: &'static str,
        file_name: Option<&'static str>,
        duration_millis: Option<u64>,
        size_bytes: Option<u64>,
        metadata_duration_millis: Option<u64>,
        parts: Vec<PlexPlayablePart>,
        expected_part_id: &'static str,
    }

    let cases = vec![
        Case {
            name: "multiple-versions-equal-duration",
            file_name: Some("Movie.1080p.mkv"),
            duration_millis: Some(7_200_000),
            size_bytes: None,
            metadata_duration_millis: Some(7_200_000),
            parts: vec![
                playable_part(
                    "version-exact",
                    Some("Movie.1080p.mkv"),
                    Some(7_200_000),
                    Some(4_000_000_000),
                ),
                playable_part(
                    "version-other",
                    Some("Movie.2160p.mkv"),
                    Some(7_200_000),
                    Some(12_000_000_000),
                ),
            ],
            expected_part_id: "version-exact",
        },
        Case {
            name: "optimized-copy-exact-size",
            file_name: Some("Movie.mkv"),
            duration_millis: Some(7_200_000),
            size_bytes: Some(8_000_000_000),
            metadata_duration_millis: Some(7_200_000),
            parts: vec![
                playable_part(
                    "original",
                    Some("Movie.mkv"),
                    Some(7_200_000),
                    Some(8_000_000_000),
                ),
                playable_part(
                    "optimized",
                    Some("Movie.mkv"),
                    Some(7_200_000),
                    Some(2_000_000_000),
                ),
            ],
            expected_part_id: "original",
        },
        Case {
            name: "missing-duration-exact-filename",
            file_name: Some("Episode.02.mkv"),
            duration_millis: None,
            size_bytes: None,
            metadata_duration_millis: None,
            parts: vec![
                playable_part("episode-01", Some("Episode.01.mkv"), None, Some(1_000)),
                playable_part("episode-02", Some("Episode.02.mkv"), None, Some(2_000)),
            ],
            expected_part_id: "episode-02",
        },
        Case {
            name: "normalized-filename",
            file_name: Some(r"D:\Shared\SHOW.S01E02.MKV"),
            duration_millis: None,
            size_bytes: None,
            metadata_duration_millis: None,
            parts: vec![
                playable_part("normalized", Some("show.s01e02.mkv"), None, Some(1_000)),
                playable_part(
                    "alternate",
                    Some("show.s01e02.optimized.mkv"),
                    None,
                    Some(900),
                ),
            ],
            expected_part_id: "normalized",
        },
        Case {
            name: "exact-filename-outranks-folded-alias",
            file_name: Some("Exact.Release.mkv"),
            duration_millis: Some(90_000),
            size_bytes: Some(1_000),
            metadata_duration_millis: None,
            parts: vec![
                playable_part(
                    "exact-case",
                    Some("Exact.Release.mkv"),
                    Some(90_000),
                    Some(1_000),
                ),
                playable_part(
                    "folded-alias",
                    Some("exact.release.MKV"),
                    Some(90_000),
                    Some(1_000),
                ),
            ],
            expected_part_id: "exact-case",
        },
        Case {
            name: "multipart-exact-part-name",
            file_name: Some("Movie.cd2.mkv"),
            duration_millis: None,
            size_bytes: None,
            metadata_duration_millis: None,
            parts: vec![
                playable_part("cd1", Some("Movie.cd1.mkv"), None, Some(1_000)),
                playable_part("cd2", Some("Movie.cd2.mkv"), None, Some(1_100)),
            ],
            expected_part_id: "cd2",
        },
        Case {
            name: "filename-outranks-duration",
            file_name: Some("Exact.Release.mkv"),
            duration_millis: Some(90_000),
            size_bytes: None,
            metadata_duration_millis: None,
            parts: vec![
                playable_part(
                    "filename",
                    Some("Exact.Release.mkv"),
                    Some(100_000),
                    Some(2_000),
                ),
                playable_part(
                    "duration",
                    Some("Other.Release.mkv"),
                    Some(90_000),
                    Some(1_000),
                ),
            ],
            expected_part_id: "filename",
        },
        Case {
            name: "filename-outranks-conflicting-size",
            file_name: Some("Exact.Release.mkv"),
            duration_millis: Some(90_000),
            size_bytes: Some(2_000),
            metadata_duration_millis: None,
            parts: vec![
                playable_part(
                    "filename",
                    Some("Exact.Release.mkv"),
                    Some(90_000),
                    Some(1_000),
                ),
                playable_part("size", Some("Other.Release.mkv"), Some(90_000), Some(2_000)),
            ],
            expected_part_id: "filename",
        },
        Case {
            name: "size-outranks-duration",
            file_name: Some("Shared.Title.mkv"),
            duration_millis: Some(90_000),
            size_bytes: Some(2_000),
            metadata_duration_millis: None,
            parts: vec![
                playable_part(
                    "size",
                    Some("Plex.Version.A.mkv"),
                    Some(100_000),
                    Some(2_000),
                ),
                playable_part(
                    "duration",
                    Some("Plex.Version.B.mkv"),
                    Some(90_000),
                    Some(1_000),
                ),
            ],
            expected_part_id: "size",
        },
    ];

    for (case_index, case) in cases.into_iter().enumerate() {
        for (order, parts) in [
            ("forward", case.parts.clone()),
            ("reverse", case.parts.iter().cloned().rev().collect()),
        ] {
            let rating_key = format!("case-{case_index}-{order}");
            assert_eq!(
                reference_choose_part(
                    &parts,
                    case.file_name,
                    case.size_bytes,
                    case.duration_millis,
                ),
                Ok(format!("/library/parts/{}/file.mkv", case.expected_part_id)),
                "{}-{order} must be uniquely decidable by the independent evidence oracle",
                case.name
            );
            let target = playlist_target(
                &rating_key,
                case.file_name,
                case.duration_millis,
                case.size_bytes,
            );
            record_mismatch(
                &mut mismatches,
                &format!("{}-{order}", case.name),
                case.expected_part_id,
                observed_part_for_playlist_target(
                    metadata(&rating_key, case.metadata_duration_millis, parts),
                    &target,
                ),
            );
        }
    }

    assert!(
        mismatches.is_empty(),
        "{DESIRED_SELECTION_INVARIANT}: {mismatches:#?}"
    );
}

#[test]
fn genuinely_indistinguishable_or_unidentified_parts_still_fail_closed() {
    let cases = [
        (
            "indistinguishable-duplicates",
            Some("Duplicate.mkv"),
            Some(90_000),
            Some(1_000),
            vec![
                playable_part(
                    "duplicate-a",
                    Some("Duplicate.mkv"),
                    Some(90_000),
                    Some(1_000),
                ),
                playable_part(
                    "duplicate-b",
                    Some("Duplicate.mkv"),
                    Some(90_000),
                    Some(1_000),
                ),
            ],
        ),
        (
            "unidentified-multipart",
            Some("Movie.mkv"),
            None,
            None,
            vec![
                playable_part("multipart-cd1", Some("Movie.cd1.mkv"), None, Some(1_000)),
                playable_part("multipart-cd2", Some("Movie.cd2.mkv"), None, Some(1_100)),
            ],
        ),
    ];

    for (rating_key, file_name, duration_millis, size_bytes, parts) in cases {
        assert!(
            reference_choose_part(&parts, file_name, size_bytes, duration_millis).is_err(),
            "{rating_key} must remain ambiguous in the independent evidence oracle"
        );
        let target = playlist_target(rating_key, file_name, duration_millis, size_bytes);
        let error = observed_part_for_playlist_target(
            metadata(rating_key, duration_millis, parts),
            &target,
        )
        .expect_err("unresolved evidence ties must remain ambiguous");
        assert!(
            error.contains("ambiguous playable parts"),
            "{rating_key} must fail closed as genuine ambiguity: {error}"
        );
    }
}

#[test]
fn duration_breaks_a_filename_and_size_tie() {
    let rating_key = "duration-breaks-evidence-tie";
    let target = playlist_target(rating_key, Some("Duplicate.mkv"), Some(90_000), Some(1_000));
    let parts = vec![
        playable_part(
            "duration-exact",
            Some("Duplicate.mkv"),
            Some(90_000),
            Some(1_000),
        ),
        playable_part(
            "duration-other",
            Some("Duplicate.mkv"),
            Some(95_000),
            Some(1_000),
        ),
    ];
    assert_eq!(
        reference_choose_part(&parts, Some("Duplicate.mkv"), Some(1_000), Some(90_000),),
        Ok("/library/parts/duration-exact/file.mkv".to_owned()),
        "the independent oracle must retain duration as the final tie-break"
    );
    let selected = observed_part_for_playlist_target(metadata(rating_key, None, parts), &target)
        .expect("duration should break a tie after filename and size evidence tie");

    assert_eq!(
        selected, "/library/parts/duration-exact/file.mkv",
        "duration remains the final deterministic discriminator"
    );
}
