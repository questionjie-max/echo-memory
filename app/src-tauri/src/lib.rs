pub mod analysis;
pub mod audio;
pub mod commands;
pub mod db;
pub mod degeneration;
pub mod dock;
pub mod document;
pub mod error;
pub mod export;
pub mod external_ai_gate;
pub mod inbox;
pub mod knowledge;
pub mod library;
pub mod memory;
pub mod record_commands;
pub mod state;
pub mod transcript;
pub mod types;
pub mod whisper;

use state::{default_library_root, AppState};
use std::path::{Path, PathBuf};
use tauri::{Builder, Manager};

fn audio_scope_root(library_root: &Path) -> PathBuf {
    library_root.join("audio")
}

#[cfg(test)]
fn is_within_audio_scope(library_root: &Path, path: &Path) -> bool {
    path.starts_with(audio_scope_root(library_root))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let library_root = default_library_root();
    let app_state = AppState::initialize(library_root.clone()).expect("初始化回声记忆资料库失败");
    Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            app.asset_protocol_scope()
                .allow_directory(audio_scope_root(&library_root), true)
                .map_err(|error| error.to_string())?;
            let handle = app.handle().clone();
            inbox::start(handle, library_root);
            Ok(())
        })
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::get_transcription_engine_status,
            commands::set_transcription_engine,
            commands::set_hf_token,
            commands::clear_hf_token,
            commands::get_record_speakers,
            commands::rename_record_speaker,
            commands::get_local_ai_status,
            commands::get_audio_preprocessor_status,
            commands::update_knowledge_settings,
            commands::pull_ollama_model,
            commands::download_whisper_model,
            commands::cancel_model_download,
            commands::create_project,
            commands::list_projects,
            commands::update_project,
            commands::delete_project,
            commands::list_analysis_templates,
            commands::create_analysis_template,
            commands::update_analysis_template,
            commands::delete_analysis_template,
            commands::list_records,
            commands::search_records,
            commands::get_knowledge_overview,
            commands::get_knowledge_index_status,
            commands::rebuild_knowledge_index,
            commands::ask_knowledge_base,
            commands::export_record,
            commands::export_knowledge_base,
            commands::create_export_ticket,
            commands::update_record_knowledge_base,
            commands::update_record_title,
            record_commands::move_records,
            record_commands::delete_records,
            commands::get_mcp_status,
            commands::set_mcp_enabled,
            commands::get_record,
            commands::record_audio_path,
            commands::list_jobs,
            commands::import_audio,
            commands::import_document,
            commands::list_transcript_segments,
            commands::list_transcript_blocks,
            commands::update_transcript_segment,
            commands::transcribe_record,
            commands::retranscribe_record,
            commands::analyze_record,
            commands::latest_analysis,
            commands::get_external_ai_settings,
            commands::update_external_ai_settings,
            commands::set_external_ai_api_key,
            commands::clear_external_ai_api_key,
            commands::test_external_ai_connection,
            commands::get_local_timeline,
            commands::get_local_growth_graph,
            commands::generate_memory_snapshot,
            commands::start_memory_generation,
            commands::get_memory_generation_job,
            commands::list_memory_snapshots,
            commands::get_memory_snapshot,
            commands::cancel_memory_generation,
            commands::update_memory_feedback,
            commands::list_memory_feedback,
            commands::get_onboarding_status,
            commands::complete_onboarding,
            commands::reset_onboarding,
            commands::suggest_watch_folders,
            commands::get_inbox_status,
            commands::add_inbox_watch_folder,
            commands::remove_inbox_watch_folder,
            commands::set_inbox_usb_detection,
            commands::rescan_inbox,
            commands::list_hotwords,
            commands::add_hotword,
            commands::remove_hotword,
            commands::get_action_dashboard,
            commands::set_action_item_status,
            commands::related_records,
            commands::correct_transcript,
            commands::get_transcript_correction_enabled,
            commands::set_transcript_correction_enabled,
            commands::get_dock_status,
            commands::ask_dock,
            commands::clear_dock_chat,
            commands::generate_template_draft,
            commands::get_output_status,
            commands::set_output_folder,
            commands::set_auto_export_analysis,
            commands::export_record_to_output,
            commands::save_dock_message_to_output,
        ])
        .run(tauri::generate_context!())
        .expect("启动回声记忆失败");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use uuid::Uuid;

    fn temporary_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("echo-audio-scope-{name}-{}", Uuid::new_v4()))
    }

    #[test]
    fn audio_scope_follows_default_and_custom_library_roots() {
        for name in ["default", "custom"] {
            let root = temporary_root(name);
            let audio_root = root.join("audio");

            assert_eq!(audio_scope_root(&root), audio_root);
            assert!(is_within_audio_scope(
                &root,
                &audio_root.join("2026/09/interview.m4a")
            ));
            assert!(!is_within_audio_scope(
                &root,
                &root.join("documents/private.txt")
            ));
            assert!(!is_within_audio_scope(
                &root,
                Path::new("/unrelated/audio/song.mp3")
            ));
        }
    }
}
