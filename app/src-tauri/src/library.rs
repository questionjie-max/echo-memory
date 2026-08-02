//! Managed local files and import service. This module never contacts a network service.

use crate::db::repository::LibraryRepository;
use crate::error::{AppError, AppResult};
use crate::types::{IngestResult, RecordBrief};
use chrono::{Datelike, Utc};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use uuid::Uuid;

const AUDIO_EXTENSIONS: [&str; 3] = ["mp3", "m4a", "wav"];

#[derive(Clone)]
pub struct ManagedLibrary {
    root: PathBuf,
    repository: LibraryRepository,
}

impl ManagedLibrary {
    pub fn open(root: impl Into<PathBuf>) -> AppResult<Self> {
        let root = root.into();
        fs::create_dir_all(root.join("audio"))?;
        fs::create_dir_all(root.join("raw"))?;
        fs::create_dir_all(root.join("exports"))?;
        Ok(Self {
            repository: LibraryRepository::new(root.join("memory.db"))?,
            root,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn repository(&self) -> &LibraryRepository {
        &self.repository
    }

    pub fn import_audio(
        &self,
        source: &Path,
        project_id: Option<&str>,
        duplicate_confirmed: bool,
    ) -> AppResult<IngestResult> {
        let extension = supported_extension(source)?;
        let hash = sha256_file(source)?;
        if let Some(existing) = self.repository.find_record_by_hash(&hash)? {
            if !duplicate_confirmed {
                return Ok(duplicate_result(existing));
            }
        }
        let duration_ms = audio_duration_ms(source, &extension)?;
        let now = Utc::now();
        let record_id = Uuid::new_v4().to_string();
        let relative_path = PathBuf::from("audio")
            .join(format!("{:04}", now.year()))
            .join(format!("{:02}", now.month()))
            .join(format!("{record_id}.{extension}"));
        let destination = self.root.join(&relative_path);
        let parent = destination
            .parent()
            .ok_or_else(|| AppError::Import("无法确定受管理目录".into()))?;
        fs::create_dir_all(parent)?;
        fs::copy(source, &destination)?;
        let title = source
            .file_stem()
            .and_then(|name| name.to_str())
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| AppError::Import("文件名无效".into()))?;
        match self.repository.create_record_with_transcription_job(
            &record_id,
            title,
            project_id,
            &relative_path,
            &hash,
            duration_ms,
        ) {
            Ok((record, _)) => Ok(IngestResult {
                record_id: record.id,
                hash,
                duplicate: false,
                duration_ms,
                title: record.title,
            }),
            Err(error) => {
                let _ = fs::remove_file(destination);
                Err(error)
            }
        }
    }
}

fn duplicate_result(record: RecordBrief) -> IngestResult {
    IngestResult {
        record_id: record.id,
        hash: record.audio_hash,
        duplicate: true,
        duration_ms: record.audio_duration_ms,
        title: record.title,
    }
}

fn supported_extension(path: &Path) -> AppResult<String> {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| AppError::Import("仅支持 MP3、M4A、WAV 文件".into()))?;
    if AUDIO_EXTENSIONS.contains(&extension.as_str()) {
        Ok(extension)
    } else {
        Err(AppError::Import("仅支持 MP3、M4A、WAV 文件".into()))
    }
}

fn sha256_file(path: &Path) -> AppResult<String> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn audio_duration_ms(path: &Path, extension: &str) -> AppResult<i64> {
    let stream = MediaSourceStream::new(Box::new(File::open(path)?), Default::default());
    let mut hint = Hint::new();
    hint.with_extension(extension);
    let probe = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|error| AppError::Import(format!("无法读取音频: {error}")))?;
    let track = probe
        .format
        .default_track()
        .ok_or_else(|| AppError::Import("音频没有可读取的轨道".into()))?;
    let frames = track
        .codec_params
        .n_frames
        .ok_or_else(|| AppError::Import("无法读取音频时长".into()))?;
    let time_base = track
        .codec_params
        .time_base
        .ok_or_else(|| AppError::Import("无法读取音频时间基准".into()))?;
    let duration = time_base.calc_time(frames);
    Ok(duration.seconds as i64 * 1_000 + (duration.frac * 1_000.0) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir()
            .join("echo_memory_library_tests")
            .join(format!("{name}_{}", COUNTER.fetch_add(1, Ordering::SeqCst)));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }
    fn write_wav(path: &Path) {
        let sample_rate = 8_000_u32;
        let samples = vec![0_u8; sample_rate as usize * 2];
        let byte_rate = sample_rate * 2;
        let file_size = 36 + samples.len() as u32;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&file_size.to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        bytes.extend_from_slice(&byte_rate.to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(samples.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&samples);
        fs::write(path, bytes).unwrap();
    }
    #[test]
    fn rejects_unsupported_extension() {
        let root = temp_root("extension");
        let source = root.join("note.txt");
        fs::write(&source, "not audio").unwrap();
        assert!(ManagedLibrary::open(root.join("library"))
            .unwrap()
            .import_audio(&source, None, false)
            .unwrap_err()
            .to_string()
            .contains("MP3"));
    }
    #[test]
    fn copies_wav_and_creates_record_and_job() {
        let root = temp_root("import");
        let source = root.join("meeting.wav");
        write_wav(&source);
        let library = ManagedLibrary::open(root.join("library")).unwrap();
        let result = library.import_audio(&source, None, false).unwrap();
        assert!(!result.duplicate);
        assert_eq!(result.duration_ms, 1_000);
        assert_eq!(
            library
                .repository()
                .list_records(None, false)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            library
                .repository()
                .list_jobs_for_record(&result.record_id)
                .unwrap()
                .len(),
            1
        );
    }
    #[test]
    fn duplicate_hash_requires_confirmation() {
        let root = temp_root("duplicate");
        let source = root.join("meeting.wav");
        write_wav(&source);
        let library = ManagedLibrary::open(root.join("library")).unwrap();
        let first = library.import_audio(&source, None, false).unwrap();
        let duplicate = library.import_audio(&source, None, false).unwrap();
        assert!(duplicate.duplicate);
        assert_eq!(duplicate.record_id, first.record_id);
        let copy = library.import_audio(&source, None, true).unwrap();
        assert_ne!(copy.record_id, first.record_id);
        assert_eq!(
            library
                .repository()
                .list_records(None, false)
                .unwrap()
                .len(),
            2
        );
    }
}
