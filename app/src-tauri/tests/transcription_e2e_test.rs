//! Opt-in local regression: import a real recording, run Whisper, then inspect SQLite state.

use echo_memory_lib::commands::transcribe_with_library;
use echo_memory_lib::library::ManagedLibrary;
use std::path::PathBuf;

#[test]
#[ignore = "requires ECHO_TEST_AUDIO and a local Whisper model"]
fn imports_transcribes_and_persists_real_local_audio() {
    let source =
        PathBuf::from(std::env::var("ECHO_TEST_AUDIO").expect("ECHO_TEST_AUDIO is required"));
    let root = std::env::temp_dir().join(format!(
        "echo-memory-transcription-e2e-{}",
        uuid::Uuid::new_v4()
    ));
    let library = ManagedLibrary::open(&root).unwrap();
    let imported = library.import_audio(&source, None, false).unwrap();

    transcribe_with_library(&library, &imported.record_id).unwrap();

    let record = library
        .repository()
        .get_record(&imported.record_id)
        .unwrap();
    let jobs = library
        .repository()
        .list_jobs_for_record(&imported.record_id)
        .unwrap();
    let segments = library
        .repository()
        .list_transcript_segments(&imported.record_id)
        .unwrap();
    let version = library
        .repository()
        .latest_transcript_version(&imported.record_id)
        .unwrap();

    assert_eq!(record.status, "completed");
    assert_eq!(jobs[0].status, "completed");
    assert!(!segments.is_empty());
    assert!(segments
        .iter()
        .all(|segment| segment.end_ms >= segment.start_ms
            && segment.normalized_text.is_some()
            && segment.normalization_version.as_deref() == Some("opencc-t2s-v1")));
    assert!(!version.model.is_empty());
    assert_eq!(version.pipeline_version, "enhanced-v2");
    assert!(!version.preprocessing_json.is_empty());

    std::fs::remove_dir_all(root).unwrap();
}
