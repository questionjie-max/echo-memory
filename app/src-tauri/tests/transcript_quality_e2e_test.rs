use echo_memory_lib::library::ManagedLibrary;
use echo_memory_lib::transcript::build_blocks;
use std::path::PathBuf;

#[test]
#[ignore = "requires ECHO_LIBRARY_ROOT and ECHO_TEST_RECORD_ID"]
fn real_record_aggregates_into_reading_blocks() {
    let root =
        PathBuf::from(std::env::var("ECHO_LIBRARY_ROOT").expect("ECHO_LIBRARY_ROOT is required"));
    let record_id = std::env::var("ECHO_TEST_RECORD_ID").expect("ECHO_TEST_RECORD_ID is required");
    let library = ManagedLibrary::open(root).unwrap();
    let segments = library
        .repository()
        .list_transcript_segments(&record_id)
        .unwrap();
    let blocks = build_blocks(&segments);
    let regular = blocks
        .iter()
        .filter(|block| (8_000..=20_000).contains(&(block.end_ms - block.start_ms)))
        .count();
    let ratio = regular as f64 / blocks.len() as f64;
    println!(
        "segments={}, blocks={}, target_ratio={ratio:.3}",
        segments.len(),
        blocks.len()
    );

    assert!(
        (110..=280).contains(&blocks.len()),
        "block count: {}",
        blocks.len()
    );
    assert!(ratio >= 0.9, "8-20 second block ratio: {ratio:.3}");
}
