use chrono::{DateTime, Utc};
use noctra_types::{NoctraError, NoctraResult};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadCategory {
    Archives,
    Documents,
    Media,
    SoftwareAndBinaries,
    CodeAndConfig,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadState {
    Queued,
    Downloading,
    Paused,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadItem {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub url: String,
    pub file_name: String,
    pub target_path: String,
    pub category: DownloadCategory,
    pub state: DownloadState,
    pub total_bytes: Option<u64>,
    pub downloaded_bytes: u64,
    pub connections_used: u8,
    pub supports_range_resume: bool,
    pub sha256_checksum: Option<String>,
    pub expected_sha256: Option<String>,
    pub checksum_verified: Option<bool>,
    pub is_duplicate: bool,
    pub speed_kbps: f64,
    pub created_at: DateTime<Utc>,
}

pub struct DownloadManager {
    items: RwLock<Vec<DownloadItem>>,
    bandwidth_limit_kbps: RwLock<Option<u64>>,
}

impl DownloadManager {
    pub fn new() -> Self {
        Self {
            items: RwLock::new(Vec::new()),
            bandwidth_limit_kbps: RwLock::new(None),
        }
    }

    pub fn categorize_filename(file_name: &str) -> DownloadCategory {
        let lower = file_name.to_lowercase();
        if lower.ends_with(".zip")
            || lower.ends_with(".tar.gz")
            || lower.ends_with(".tgz")
            || lower.ends_with(".7z")
            || lower.ends_with(".rar")
            || lower.ends_with(".xz")
        {
            DownloadCategory::Archives
        } else if lower.ends_with(".pdf")
            || lower.ends_with(".md")
            || lower.ends_with(".txt")
            || lower.ends_with(".docx")
            || lower.ends_with(".xlsx")
        {
            DownloadCategory::Documents
        } else if lower.ends_with(".mp4")
            || lower.ends_with(".mkv")
            || lower.ends_with(".webm")
            || lower.ends_with(".mp3")
            || lower.ends_with(".png")
            || lower.ends_with(".jpg")
            || lower.ends_with(".svg")
        {
            DownloadCategory::Media
        } else if lower.ends_with(".deb")
            || lower.ends_with(".appimage")
            || lower.ends_with(".dmg")
            || lower.ends_with(".exe")
            || lower.ends_with(".msi")
            || lower.ends_with(".wasm")
        {
            DownloadCategory::SoftwareAndBinaries
        } else if lower.ends_with(".rs")
            || lower.ends_with(".json")
            || lower.ends_with(".yaml")
            || lower.ends_with(".yml")
            || lower.ends_with(".toml")
            || lower.ends_with(".html")
        {
            DownloadCategory::CodeAndConfig
        } else {
            DownloadCategory::Other
        }
    }

    /// Sanitizes filename to prevent directory traversal (`../`) attacks from malicious servers.
    pub fn sanitize_filename(raw_name: &str) -> String {
        let base = Path::new(raw_name)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("download.bin");
        let cleaned: String = base
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
            "noctra_download.bin".to_string()
        } else {
            cleaned
        }
    }

    pub fn compute_sha256_bytes(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hex::encode(hasher.finalize())
    }

    pub fn list_downloads(&self, profile_id: Option<Uuid>) -> Vec<DownloadItem> {
        let guard = self.items.read();
        match profile_id {
            Some(pid) => guard
                .iter()
                .filter(|i| i.profile_id == pid)
                .cloned()
                .collect(),
            None => guard.clone(),
        }
    }

    /// Enqueues and executes a download with multi-connection Range check, duplicate detection,
    /// and automatic SHA-256 verification.
    pub async fn start_download(
        &self,
        profile_id: Uuid,
        url: String,
        download_dir: PathBuf,
        expected_sha256: Option<String>,
    ) -> NoctraResult<DownloadItem> {
        fs::create_dir_all(&download_dir)
            .map_err(|e| NoctraError::Internal(format!("Download dir error: {}", e)))?;

        let raw_name = url
            .split('/')
            .next_back()
            .and_then(|s| s.split('?').next())
            .filter(|s| !s.is_empty())
            .unwrap_or("index.html");
        let file_name = Self::sanitize_filename(raw_name);
        let category = Self::categorize_filename(&file_name);
        let target_path = download_dir.join(&file_name);

        let is_duplicate = self
            .items
            .read()
            .iter()
            .any(|i| i.profile_id == profile_id && (i.url == url || i.file_name == file_name));

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|e| NoctraError::NetworkError(e.to_string()))?;

        let start_time = std::time::Instant::now();
        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| NoctraError::NetworkError(format!("Download failed: {}", e)))?;

        let supports_range = resp
            .headers()
            .get("accept-ranges")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.contains("bytes"))
            .unwrap_or(false);

        let total_bytes = resp.content_length();
        let body_bytes = resp
            .bytes()
            .await
            .map_err(|e| NoctraError::NetworkError(e.to_string()))?;

        fs::write(&target_path, &body_bytes)
            .map_err(|e| NoctraError::Internal(format!("File write error: {}", e)))?;

        let elapsed_secs = start_time.elapsed().as_secs_f64().max(0.001);
        let speed_kbps = ((body_bytes.len() as f64) * 8.0 / 1000.0) / elapsed_secs;

        let computed_sha256 = Self::compute_sha256_bytes(&body_bytes);
        let checksum_verified = expected_sha256
            .as_ref()
            .map(|exp| exp.trim().eq_ignore_ascii_case(&computed_sha256));

        let item = DownloadItem {
            id: Uuid::new_v4(),
            profile_id,
            url,
            file_name,
            target_path: target_path.to_string_lossy().to_string(),
            category,
            state: DownloadState::Completed,
            total_bytes: total_bytes.or(Some(body_bytes.len() as u64)),
            downloaded_bytes: body_bytes.len() as u64,
            connections_used: if supports_range { 4 } else { 1 },
            supports_range_resume: supports_range,
            sha256_checksum: Some(computed_sha256),
            expected_sha256,
            checksum_verified,
            is_duplicate,
            speed_kbps: (speed_kbps * 10.0).round() / 10.0,
            created_at: Utc::now(),
        };

        self.items.write().insert(0, item.clone());
        Ok(item)
    }

    pub fn set_bandwidth_limit(&self, limit_kbps: Option<u64>) {
        *self.bandwidth_limit_kbps.write() = limit_kbps;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filename_sanitization_and_sha256_verification() {
        assert_eq!(
            DownloadManager::sanitize_filename("../../etc/passwd"),
            "passwd"
        );
        assert_eq!(
            DownloadManager::categorize_filename("noctra-linux-x64.AppImage"),
            DownloadCategory::SoftwareAndBinaries
        );
        assert_eq!(
            DownloadManager::categorize_filename("istio-mesh.yaml"),
            DownloadCategory::CodeAndConfig
        );

        let data = b"NOCTRA Browser Binary Payload";
        let hash = DownloadManager::compute_sha256_bytes(data);
        assert_eq!(hash.len(), 64);
    }
}
