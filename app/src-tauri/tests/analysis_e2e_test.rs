//! Opt-in local regression against an existing record and local Ollama model.

use echo_memory_lib::analysis::{resolve_citation_aliases, verify_citations, AnalysisDraft};
use echo_memory_lib::commands::{analyze_with_library, revalidate_analysis_with_library};
use echo_memory_lib::library::ManagedLibrary;
use std::path::PathBuf;

#[test]
#[ignore = "requires ECHO_LIBRARY_ROOT, ECHO_TEST_RECORD_ID and local Ollama"]
fn analyzes_long_record_with_key_points_and_verified_citations() {
    let root =
        PathBuf::from(std::env::var("ECHO_LIBRARY_ROOT").expect("ECHO_LIBRARY_ROOT is required"));
    let record_id = std::env::var("ECHO_TEST_RECORD_ID").expect("ECHO_TEST_RECORD_ID is required");
    let library = ManagedLibrary::open(root).unwrap();

    analyze_with_library(&library, &record_id).unwrap();

    let stored = library
        .repository()
        .latest_analysis(&record_id)
        .unwrap()
        .unwrap();
    let draft: AnalysisDraft = serde_json::from_str(&stored.content_json).unwrap();
    assert!((3..=8).contains(&draft.key_points.len()));
    assert!(draft
        .key_points
        .iter()
        .all(|item| !item.citation_segment_ids.is_empty() && item.start_ms.is_some()));
    assert_eq!(stored.status, "completed", "{:?}", draft.quality_warning);
}

#[test]
#[ignore = "requires ECHO_LIBRARY_ROOT and ECHO_TEST_RECORD_ID"]
fn revalidates_latest_local_quotes_against_transcript() {
    let root =
        PathBuf::from(std::env::var("ECHO_LIBRARY_ROOT").expect("ECHO_LIBRARY_ROOT is required"));
    let record_id = std::env::var("ECHO_TEST_RECORD_ID").expect("ECHO_TEST_RECORD_ID is required");
    let library = ManagedLibrary::open(root).unwrap();
    let stored = library
        .repository()
        .latest_analysis(&record_id)
        .unwrap()
        .unwrap();
    let segments = library
        .repository()
        .list_transcript_segments(&record_id)
        .unwrap();
    let mut draft: AnalysisDraft = serde_json::from_str(&stored.content_json).unwrap();
    resolve_citation_aliases(&mut draft, &segments);
    let verified = verify_citations(&draft, &segments);
    assert!(verified
        .key_points
        .iter()
        .all(|item| !item.citation_segment_ids.is_empty() && item.start_ms.is_some()));
}

#[test]
#[ignore = "requires ECHO_LIBRARY_ROOT and ECHO_TEST_ANALYSIS_ID"]
fn saves_a_fully_revalidated_analysis() {
    let root =
        PathBuf::from(std::env::var("ECHO_LIBRARY_ROOT").expect("ECHO_LIBRARY_ROOT is required"));
    let analysis_id =
        std::env::var("ECHO_TEST_ANALYSIS_ID").expect("ECHO_TEST_ANALYSIS_ID is required");
    let library = ManagedLibrary::open(root).unwrap();
    let stored = revalidate_analysis_with_library(&library, &analysis_id).unwrap();
    assert_eq!(stored.status, "completed");
}
