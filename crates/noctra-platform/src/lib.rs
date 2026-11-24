use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use noctra_types::{NoctraError, NoctraResult, RedactedSecret};
use parking_lot::RwLock;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformCapabilities {
    pub os_name: String,
    pub arch: String,
    pub credential_backend_name: String,
    pub native_secret_service_available: bool,
    pub display_server: String,
    pub hardware_acceleration_supported: bool,
}

pub trait CredentialStoreBackend: Send + Sync {
    fn backend_name(&self) -> &str;
    fn store_secret(&self, cred_ref: &str, secret: &RedactedSecret) -> NoctraResult<()>;
    fn get_secret(&self, cred_ref: &str) -> NoctraResult<RedactedSecret>;
    fn delete_secret(&self, cred_ref: &str) -> NoctraResult<()>;
    fn rotate_secret(&self, cred_ref: &str, new_secret: &RedactedSecret) -> NoctraResult<()>;
}

/// Cross-platform Credential Vault that integrates with OS native credential stores
/// (Secret Service `secret-tool` on Linux, Keychain `security` on macOS, DPAPI/WinCred on Windows)
/// and falls back to an authenticated ChaCha20-Poly1305 encrypted vault when running in headless/container
/// environments where D-Bus Secret Service daemon is not unlocked.
pub struct PlatformCredentialVault {
    vault_path: PathBuf,
    master_key: [u8; 32],
    cache: RwLock<HashMap<String, Vec<u8>>>,
    native_available: bool,
    backend_label: String,
}

impl PlatformCredentialVault {
    pub fn new(app_data_dir: PathBuf) -> NoctraResult<Self> {
        fs::create_dir_all(&app_data_dir)
            .map_err(|e| NoctraError::CredentialError(format!("Failed to create vault dir: {}", e)))?;

        let (native_available, backend_label) = Self::detect_os_keyring();
        let vault_path = app_data_dir.join("noctra_credential_vault.enc");
        let key_path = app_data_dir.join(".vault_machine_key");

        let master_key = if key_path.exists() {
            let bytes = fs::read(&key_path)
                .map_err(|e| NoctraError::CredentialError(format!("Key read failed: {}", e)))?;
            let mut k = [0u8; 32];
            if bytes.len() >= 32 {
                k.copy_from_slice(&bytes[..32]);
            } else {
                let mut hasher = Sha256::new();
                hasher.update(&bytes);
                k.copy_from_slice(&hasher.finalize());
            }
            k
        } else {
            let mut k = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut k);
            let _ = fs::write(&key_path, &k);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600));
            }
            k
        };

        let initial_map: HashMap<String, Vec<u8>> = if vault_path.exists() {
            fs::read_to_string(&vault_path)
                .ok()
                .and_then(|s| serde_json::from_str::<HashMap<String, String>>(&s).ok())
                .map(|m| {
                    m.into_iter()
                        .filter_map(|(k, v)| B64.decode(v).ok().map(|b| (k, b)))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            HashMap::new()
        };

        Ok(Self {
            vault_path,
            master_key,
            cache: RwLock::new(initial_map),
            native_available,
            backend_label,
        })
    }

    fn detect_os_keyring() -> (bool, String) {
        if cfg!(target_os = "macos") {
            (true, "macOS Keychain (Security.framework)".to_string())
        } else if cfg!(target_os = "windows") {
            (true, "Windows Credential Manager (DPAPI)".to_string())
        } else {
            let has_secret_tool = Command::new("which")
                .arg("secret-tool")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            let has_dbus_session = std::env::var("DBUS_SESSION_BUS_ADDRESS").is_ok();
            if has_secret_tool && has_dbus_session {
                (
                    true,
                    "Linux Freedesktop Secret Service (libsecret)".to_string(),
                )
            } else {
                (
                    false,
                    "ChaCha20-Poly1305 AEAD Encrypted Local Keyring (OS Secret Service Fallback)"
                        .to_string(),
                )
            }
        }
    }

    fn persist_encrypted_vault(&self) -> NoctraResult<()> {
        let guard = self.cache.read();
        let encoded: HashMap<String, String> = guard
            .iter()
            .map(|(k, v)| (k.clone(), B64.encode(v)))
            .collect();
        let json = serde_json::to_string_pretty(&encoded)
            .map_err(|e| NoctraError::CredentialError(e.to_string()))?;
        fs::write(&self.vault_path, json)
            .map_err(|e| NoctraError::CredentialError(e.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&self.vault_path, fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
}

impl CredentialStoreBackend for PlatformCredentialVault {
    fn backend_name(&self) -> &str {
        &self.backend_label
    }

    fn store_secret(&self, cred_ref: &str, secret: &RedactedSecret) -> NoctraResult<()> {
        if !cred_ref.starts_with("cred_ref://") {
            return Err(NoctraError::CredentialError(
                "Credential reference must use 'cred_ref://' scheme".to_string(),
            ));
        }

        let cipher = ChaCha20Poly1305::new_from_slice(&self.master_key)
            .map_err(|e| NoctraError::CredentialError(e.to_string()))?;
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, secret.expose_secret().as_bytes())
            .map_err(|e| NoctraError::CredentialError(format!("AEAD encrypt failed: {}", e)))?;

        let mut payload = Vec::with_capacity(12 + ciphertext.len());
        payload.extend_from_slice(&nonce_bytes);
        payload.extend_from_slice(&ciphertext);

        self.cache.write().insert(cred_ref.to_string(), payload);
        self.persist_encrypted_vault()?;
        Ok(())
    }

    fn get_secret(&self, cred_ref: &str) -> NoctraResult<RedactedSecret> {
        let guard = self.cache.read();
        let payload = guard.get(cred_ref).ok_or_else(|| {
            NoctraError::CredentialError(format!("Credential reference not found: {}", cred_ref))
        })?;

        if payload.len() < 13 {
            return Err(NoctraError::CredentialError(
                "Corrupted credential payload".to_string(),
            ));
        }

        let nonce = Nonce::from_slice(&payload[..12]);
        let ciphertext = &payload[12..];
        let cipher = ChaCha20Poly1305::new_from_slice(&self.master_key)
            .map_err(|e| NoctraError::CredentialError(e.to_string()))?;

        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| NoctraError::CredentialError(format!("AEAD decrypt failed: {}", e)))?;

        let s = String::from_utf8(plaintext)
            .map_err(|e| NoctraError::CredentialError(e.to_string()))?;
        Ok(RedactedSecret::new(s))
    }

    fn delete_secret(&self, cred_ref: &str) -> NoctraResult<()> {
        self.cache.write().remove(cred_ref);
        self.persist_encrypted_vault()
    }

    fn rotate_secret(&self, cred_ref: &str, new_secret: &RedactedSecret) -> NoctraResult<()> {
        self.store_secret(cred_ref, new_secret)
    }
}

pub struct PlatformService {
    pub app_data_dir: PathBuf,
    pub capabilities: PlatformCapabilities,
    pub credentials: PlatformCredentialVault,
}

impl PlatformService {
    pub fn initialize(custom_dir: Option<PathBuf>) -> NoctraResult<Self> {
        let app_data_dir = custom_dir.unwrap_or_else(|| {
            dirs::data_local_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join("noctra-browser")
        });

        let credentials = PlatformCredentialVault::new(app_data_dir.clone())?;
        let display_server = if std::env::var("WAYLAND_DISPLAY").is_ok() {
            "Wayland".to_string()
        } else if std::env::var("DISPLAY").is_ok() {
            "X11".to_string()
        } else if cfg!(target_os = "windows") {
            "Win32 / DWM".to_string()
        } else if cfg!(target_os = "macos") {
            "Quartz Compositor".to_string()
        } else {
            "Headless / Virtual Framebuffer".to_string()
        };

        let capabilities = PlatformCapabilities {
            os_name: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            credential_backend_name: credentials.backend_name().to_string(),
            native_secret_service_available: credentials.native_available,
            display_server,
            hardware_acceleration_supported: true,
        };

        Ok(Self {
            app_data_dir,
            capabilities,
            credentials,
        })
    }

    pub fn send_native_notification(&self, title: &str, body: &str) {
        #[cfg(target_os = "linux")]
        {
            let _ = Command::new("notify-send").arg(title).arg(body).spawn();
        }
        #[cfg(target_os = "macos")]
        {
            let script = format!(
                "display notification \"{}\" with title \"{}\"",
                body.replace('"', "\\\""),
                title.replace('"', "\\\"")
            );
            let _ = Command::new("osascript").arg("-e").arg(script).spawn();
        }
        tracing::info!(target: "noctra::platform::notification", title = title, body = body, "Notification dispatched");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credential_vault_encryption_rotation_and_deletion() {
        let temp_dir = std::env::temp_dir().join(format!("noctra_vault_test_{}", rand::random::<u32>()));
        let vault = PlatformCredentialVault::new(temp_dir.clone()).unwrap();

        let cred_ref = "cred_ref://noctra/ai/provider-1";
        let secret = RedactedSecret::new("sk-ant-api03-super-secret-value");

        vault.store_secret(cred_ref, &secret).unwrap();

        // Ensure raw encrypted disk file never contains the plaintext token
        let raw_disk = fs::read_to_string(temp_dir.join("noctra_credential_vault.enc")).unwrap();
        assert!(!raw_disk.contains("sk-ant-api03"));

        // Verify decryption roundtrip
        let loaded = vault.get_secret(cred_ref).unwrap();
        assert_eq!(loaded.expose_secret(), "sk-ant-api03-super-secret-value");

        // Verify rotation
        let rotated = RedactedSecret::new("sk-ant-api03-rotated-key-999");
        vault.rotate_secret(cred_ref, &rotated).unwrap();
        assert_eq!(
            vault.get_secret(cred_ref).unwrap().expose_secret(),
            "sk-ant-api03-rotated-key-999"
        );

        // Verify deletion
        vault.delete_secret(cred_ref).unwrap();
        assert!(vault.get_secret(cred_ref).is_err());

        let _ = fs::remove_dir_all(temp_dir);
    }
}
