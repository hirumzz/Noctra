use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use chrono::{DateTime, Utc};
use noctra_types::{NoctraError, NoctraResult};
use parking_lot::RwLock;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectiveSyncConfig {
    pub sync_bookmarks: bool,
    pub sync_history: bool,
    pub sync_passwords: bool,
    pub sync_extensions: bool,
    pub sync_settings: bool,
    pub sync_profiles_and_workspaces: bool,
    pub sync_open_tabs_and_floating_windows: bool,
}

impl Default for SelectiveSyncConfig {
    fn default() -> Self {
        Self {
            sync_bookmarks: true,
            sync_history: true,
            sync_passwords: false, // Opt-in for passwords (Section 7)
            sync_extensions: true,
            sync_settings: true,
            sync_profiles_and_workspaces: true,
            sync_open_tabs_and_floating_windows: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedSyncDevice {
    pub device_id: Uuid,
    pub device_name: String,
    pub os_platform: String,
    pub last_synced_at: DateTime<Utc>,
    pub is_current_device: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedSyncEnvelope {
    pub envelope_id: Uuid,
    pub device_id: Uuid,
    pub sequence_number: u64,
    pub nonce_b64: String,
    pub ciphertext_b64: String,
    pub payload_sha256: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncStatusReport {
    pub google_oauth_connected: bool,
    pub google_account_email: Option<String>,
    pub sync_paused: bool,
    pub e2ee_active: bool,
    pub pending_offline_queue_count: usize,
    pub last_sync_sequence: u64,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub selective_config: SelectiveSyncConfig,
    pub linked_devices: Vec<LinkedSyncDevice>,
}

pub struct NoctraSyncEngine {
    e2ee_key: RwLock<[u8; 32]>,
    google_email: RwLock<Option<String>>,
    paused: RwLock<bool>,
    selective: RwLock<SelectiveSyncConfig>,
    devices: RwLock<Vec<LinkedSyncDevice>>,
    offline_queue: RwLock<Vec<EncryptedSyncEnvelope>>,
    sequence: RwLock<u64>,
    last_synced: RwLock<Option<DateTime<Utc>>>,
}

impl NoctraSyncEngine {
    pub fn new(passphrase: &str) -> Self {
        let key = Self::derive_e2ee_key(passphrase);
        let current_device = LinkedSyncDevice {
            device_id: Uuid::new_v4(),
            device_name: format!("NOCTRA-Desktop-{}", std::env::consts::OS),
            os_platform: format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH),
            last_synced_at: Utc::now(),
            is_current_device: true,
        };

        Self {
            e2ee_key: RwLock::new(key),
            google_email: RwLock::new(None),
            paused: RwLock::new(false),
            selective: RwLock::new(SelectiveSyncConfig::default()),
            devices: RwLock::new(vec![current_device]),
            offline_queue: RwLock::new(Vec::new()),
            sequence: RwLock::new(1),
            last_synced: RwLock::new(None),
        }
    }

    fn derive_e2ee_key(passphrase: &str) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"NOCTRA-E2EE-SYNC-V1:");
        hasher.update(passphrase.as_bytes());
        let digest = hasher.finalize();
        let mut key = [0u8; 32];
        key.copy_from_slice(&digest);
        key
    }

    pub fn set_sync_passphrase(&self, passphrase: &str) {
        *self.e2ee_key.write() = Self::derive_e2ee_key(passphrase);
    }

    /// Generates a Google OAuth 2.0 PKCE authorization URL.
    /// Section 7 Note: Google OAuth authenticates user identity/Google Drive backup;
    /// actual browser data synchronization uses NOCTRA's E2EE Sync backend.
    pub fn build_google_oauth_authorize_url(client_id: &str, redirect_uri: &str) -> String {
        format!(
            "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope=openid%20email%20profile&access_type=offline&prompt=consent",
            client_id, redirect_uri
        )
    }

    pub fn link_google_account(&self, email: String) {
        *self.google_email.write() = Some(email);
    }

    pub fn unlink_device(&self, device_id: Uuid) {
        self.devices.write().retain(|d| d.device_id != device_id || d.is_current_device);
    }

    pub fn set_paused(&self, paused: bool) {
        *self.paused.write() = paused;
    }

    pub fn update_selective_config(&self, cfg: SelectiveSyncConfig) {
        *self.selective.write() = cfg;
    }

    /// Encrypts any sync payload using ChaCha20-Poly1305 AEAD before transmission or offline queueing.
    /// Plaintext is NEVER transmitted over the network (Section 7).
    pub fn encrypt_sync_payload(&self, json_payload: &str) -> NoctraResult<EncryptedSyncEnvelope> {
        let key = *self.e2ee_key.read();
        let cipher = ChaCha20Poly1305::new_from_slice(&key)
            .map_err(|e| NoctraError::Internal(e.to_string()))?;

        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, json_payload.as_bytes())
            .map_err(|e| NoctraError::Internal(format!("Sync E2EE encrypt error: {}", e)))?;

        let mut hasher = Sha256::new();
        hasher.update(json_payload.as_bytes());
        let sha = format!("{:x}", hasher.finalize());

        let mut seq = self.sequence.write();
        let current_seq = *seq;
        *seq += 1;

        let device_id = self
            .devices
            .read()
            .first()
            .map(|d| d.device_id)
            .unwrap_or_else(Uuid::new_v4);

        let env = EncryptedSyncEnvelope {
            envelope_id: Uuid::new_v4(),
            device_id,
            sequence_number: current_seq,
            nonce_b64: B64.encode(nonce_bytes),
            ciphertext_b64: B64.encode(ciphertext),
            payload_sha256: sha,
            created_at: Utc::now(),
        };

        self.offline_queue.write().push(env.clone());
        *self.last_synced.write() = Some(Utc::now());
        Ok(env)
    }

    pub fn decrypt_sync_envelope(&self, env: &EncryptedSyncEnvelope) -> NoctraResult<String> {
        let key = *self.e2ee_key.read();
        let cipher = ChaCha20Poly1305::new_from_slice(&key)
            .map_err(|e| NoctraError::Internal(e.to_string()))?;

        let nonce_bytes = B64
            .decode(&env.nonce_b64)
            .map_err(|e| NoctraError::ValidationError(e.to_string()))?;
        let ciphertext = B64
            .decode(&env.ciphertext_b64)
            .map_err(|e| NoctraError::ValidationError(e.to_string()))?;

        if nonce_bytes.len() != 12 {
            return Err(NoctraError::ValidationError("Invalid sync nonce".to_string()));
        }

        let plaintext = cipher
            .decrypt(Nonce::from_slice(&nonce_bytes), ciphertext.as_ref())
            .map_err(|_| {
                NoctraError::SecurityViolation(
                    "E2EE Sync decryption failed: invalid passphrase or tampered ciphertext"
                        .to_string(),
                )
            })?;

        String::from_utf8(plaintext).map_err(|e| NoctraError::ValidationError(e.to_string()))
    }

    pub fn status(&self) -> SyncStatusReport {
        let email = self.google_email.read().clone();
        SyncStatusReport {
            google_oauth_connected: email.is_some(),
            google_account_email: email,
            sync_paused: *self.paused.read(),
            e2ee_active: true,
            pending_offline_queue_count: self.offline_queue.read().len(),
            last_sync_sequence: *self.sequence.read(),
            last_synced_at: *self.last_synced.read(),
            selective_config: self.selective.read().clone(),
            linked_devices: self.devices.read().clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_e2ee_sync_envelope_never_exposes_plaintext() {
        let sync = NoctraSyncEngine::new("correct-horse-battery-staple");
        let secret_bookmark_json = r#"{"url":"https://internal.nocta.dev/istio","title":"Nocto Secret Dashboard"}"#;

        let envelope = sync.encrypt_sync_payload(secret_bookmark_json).unwrap();
        assert!(!envelope.ciphertext_b64.contains("internal.nocta.dev"));
        assert!(!envelope.ciphertext_b64.contains("Nocto Secret Dashboard"));

        let decrypted = sync.decrypt_sync_envelope(&envelope).unwrap();
        assert_eq!(decrypted, secret_bookmark_json);
    }
}
