pub mod analysis;
pub mod audio;
pub mod commands;
pub mod db;
pub mod error;
pub mod export;
pub mod knowledge;
pub mod library;
pub mod state;
pub mod transcript;
pub mod types;
pub mod whisper;

use state::{default_library_root, AppState};
use tauri::Builder;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app_state = AppState::initialize(default_library_root()).expect("初始化回声记忆资料库失败");
    Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            commands::greet,
            commands::app_info,
            commands::generate_id,
            commands::get_local_ai_status,
            commands::get_audio_preprocessor_status,
            commands::update_knowledge_settings,
            commands::pull_ollama_model,
            commands::download_whisper_model,
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
            commands::update_record_knowledge_base,
            commands::update_record_title,
            commands::get_mcp_status,
            commands::set_mcp_enabled,
            commands::get_record,
            commands::record_audio_path,
            commands::list_jobs,
            commands::import_audio,
            commands::list_transcript_segments,
            commands::list_transcript_blocks,
            commands::update_transcript_segment,
            commands::transcribe_record,
            commands::retranscribe_record,
            commands::analyze_record,
            commands::latest_analysis,
        ])
        .run(tauri::generate_context!())
        .expect("启动回声记忆失败");
}
