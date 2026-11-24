use chrono::Utc;
use noctra_database::NoctraDatabase;
use noctra_types::{
    BrowserProfile, NoctraError, NoctraResult, ProfilePreset, WindowLayoutMode, Workspace,
    WorkspaceSessionMode,
};
use parking_lot::RwLock;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use url::Url;
use uuid::Uuid;

pub struct ProfileAndWorkspaceEngine {
    base_dir: PathBuf,
    db: Arc<NoctraDatabase>,
    profiles: RwLock<Vec<BrowserProfile>>,
    workspaces: RwLock<Vec<Workspace>>,
    active_profile_id: RwLock<Uuid>,
    active_workspace_id: RwLock<Uuid>,
}

impl ProfileAndWorkspaceEngine {
    pub fn initialize(base_dir: PathBuf, db: Arc<NoctraDatabase>) -> NoctraResult<Self> {
        let profiles_dir = base_dir.join("profiles");
        fs::create_dir_all(&profiles_dir)
            .map_err(|e| NoctraError::ProfileError(format!("Dir creation failed: {}", e)))?;

        let mut loaded_profiles = db.load_profiles()?;
        let mut loaded_workspaces = db.load_workspaces()?;

        if loaded_profiles.is_empty() {
            loaded_profiles = Self::create_default_profiles(&profiles_dir)?;
            db.save_profiles(&loaded_profiles)?;
        }

        let primary_profile_id = loaded_profiles[0].id;

        if loaded_workspaces.is_empty() {
            loaded_workspaces = Self::create_default_workspaces(&loaded_profiles);
            db.save_workspaces(&loaded_workspaces)?;
        }

        let primary_workspace_id = loaded_workspaces
            .iter()
            .find(|w| w.profile_id == primary_profile_id)
            .map(|w| w.id)
            .unwrap_or_else(|| loaded_workspaces[0].id);

        Ok(Self {
            base_dir,
            db,
            profiles: RwLock::new(loaded_profiles),
            workspaces: RwLock::new(loaded_workspaces),
            active_profile_id: RwLock::new(primary_profile_id),
            active_workspace_id: RwLock::new(primary_workspace_id),
        })
    }

    fn create_default_profiles(profiles_dir: &PathBuf) -> NoctraResult<Vec<BrowserProfile>> {
        let presets = vec![
            (
                "Personal",
                ProfilePreset::Personal,
                "user",
                "#8B7CF6",
                "https://en.wikipedia.org/wiki/Rust_(programming_language)",
            ),
            (
                "Work",
                ProfilePreset::Work,
                "briefcase",
                "#3B82F6",
                "https://news.ycombinator.com",
            ),
            (
                "Development",
                ProfilePreset::Development,
                "terminal",
                "#10B981",
                "https://doc.rust-lang.org",
            ),
            (
                "Testing",
                ProfilePreset::Testing,
                "flask",
                "#F59E0B",
                "https://httpbin.org",
            ),
            (
                "Guest",
                ProfilePreset::Guest,
                "shield",
                "#A1A1AA",
                "https://duckduckgo.com",
            ),
        ];

        let mut list = Vec::new();
        for (name, preset, icon, color, startup_url) in presets {
            let id = Uuid::new_v4();
            let partition_dir = profiles_dir.join(id.to_string()).join("chromium_partition");
            let download_dir = profiles_dir.join(id.to_string()).join("downloads");
            let _ = fs::create_dir_all(&partition_dir);
            let _ = fs::create_dir_all(&download_dir);

            list.push(BrowserProfile {
                id,
                name: name.to_string(),
                preset,
                icon: icon.to_string(),
                color: color.to_string(),
                startup_url: startup_url.to_string(),
                default_search_engine: "Google".to_string(),
                storage_partition_path: partition_dir.to_string_lossy().to_string(),
                cookie_namespace: format!("cookie_ns_profile_{}", id),
                download_directory: download_dir.to_string_lossy().to_string(),
                proxy_url: None,
                wireguard_enabled: false,
                tor_mode_enabled: false,
                created_at: Utc::now(),
            });
        }
        Ok(list)
    }

    fn create_default_workspaces(profiles: &[BrowserProfile]) -> Vec<Workspace> {
        let mut workspaces = Vec::new();
        for profile in profiles {
            let pid = profile.id;
            workspaces.push(Workspace {
                id: Uuid::new_v4(),
                profile_id: pid,
                name: "General Space".to_string(),
                icon: "compass".to_string(),
                accent_color: profile.color.clone(),
                session_mode: WorkspaceSessionMode::SharedSession,
                storage_partition_id: format!("part_shared_{}", pid),
                startup_pages: vec![profile.startup_url.clone()],
                domain_routing_rules: vec![],
                proxy_override: None,
                layout_mode: WindowLayoutMode::FreeFloating,
                created_at: Utc::now(),
            });

            workspaces.push(Workspace {
                id: Uuid::new_v4(),
                profile_id: pid,
                name: "DevOps & SRE".to_string(),
                icon: "server".to_string(),
                accent_color: "#10B981".to_string(),
                session_mode: WorkspaceSessionMode::SharedSession,
                storage_partition_id: format!("part_shared_{}", pid),
                startup_pages: vec![
                    "https://kubernetes.io".to_string(),
                    "https://istio.io".to_string(),
                ],
                domain_routing_rules: vec![
                    "github.com".to_string(),
                    "gitlab.com".to_string(),
                    "kubernetes.io".to_string(),
                    "istio.io".to_string(),
                    "console.cloud.google.com".to_string(),
                ],
                proxy_override: None,
                layout_mode: WindowLayoutMode::TileTwoVertical,
                created_at: Utc::now(),
            });

            let isolated_id = Uuid::new_v4();
            workspaces.push(Workspace {
                id: isolated_id,
                profile_id: pid,
                name: "Isolated Sandbox".to_string(),
                icon: "lock".to_string(),
                accent_color: "#F59E0B".to_string(),
                session_mode: WorkspaceSessionMode::Isolated,
                storage_partition_id: format!("part_isolated_{}_{}", pid, isolated_id),
                startup_pages: vec!["https://duckduckgo.com".to_string()],
                domain_routing_rules: vec!["temp-mail.org".to_string()],
                proxy_override: None,
                layout_mode: WindowLayoutMode::FreeFloating,
                created_at: Utc::now(),
            });
        }
        workspaces
    }

    pub fn list_profiles(&self) -> Vec<BrowserProfile> {
        self.profiles.read().clone()
    }

    pub fn active_profile(&self) -> NoctraResult<BrowserProfile> {
        let active_id = *self.active_profile_id.read();
        self.profiles
            .read()
            .iter()
            .find(|p| p.id == active_id)
            .cloned()
            .ok_or_else(|| NoctraError::ProfileError("Active profile not found".to_string()))
    }

    pub fn create_profile(
        &self,
        name: String,
        color: String,
        icon: String,
        startup_url: String,
    ) -> NoctraResult<BrowserProfile> {
        let id = Uuid::new_v4();
        let profiles_dir = self.base_dir.join("profiles");
        let partition_dir = profiles_dir.join(id.to_string()).join("chromium_partition");
        let download_dir = profiles_dir.join(id.to_string()).join("downloads");
        fs::create_dir_all(&partition_dir)
            .map_err(|e| NoctraError::ProfileError(e.to_string()))?;
        fs::create_dir_all(&download_dir)
            .map_err(|e| NoctraError::ProfileError(e.to_string()))?;

        let profile = BrowserProfile {
            id,
            name: name.clone(),
            preset: ProfilePreset::Custom,
            icon,
            color: color.clone(),
            startup_url: startup_url.clone(),
            default_search_engine: "Google".to_string(),
            storage_partition_path: partition_dir.to_string_lossy().to_string(),
            cookie_namespace: format!("cookie_ns_profile_{}", id),
            download_directory: download_dir.to_string_lossy().to_string(),
            proxy_url: None,
            wireguard_enabled: false,
            tor_mode_enabled: false,
            created_at: Utc::now(),
        };

        let default_ws = Workspace {
            id: Uuid::new_v4(),
            profile_id: id,
            name: format!("{} Main", name),
            icon: "layers".to_string(),
            accent_color: color,
            session_mode: WorkspaceSessionMode::SharedSession,
            storage_partition_id: format!("part_shared_{}", id),
            startup_pages: vec![startup_url],
            domain_routing_rules: vec![],
            proxy_override: None,
            layout_mode: WindowLayoutMode::FreeFloating,
            created_at: Utc::now(),
        };

        {
            let mut p_guard = self.profiles.write();
            p_guard.push(profile.clone());
            self.db.save_profiles(&p_guard)?;
        }
        {
            let mut w_guard = self.workspaces.write();
            w_guard.push(default_ws);
            self.db.save_workspaces(&w_guard)?;
        }
        Ok(profile)
    }

    pub fn switch_profile(&self, profile_id: Uuid) -> NoctraResult<(BrowserProfile, Workspace)> {
        let profile = self
            .profiles
            .read()
            .iter()
            .find(|p| p.id == profile_id)
            .cloned()
            .ok_or_else(|| NoctraError::ProfileError("Profile not found".to_string()))?;

        let ws = self
            .workspaces
            .read()
            .iter()
            .find(|w| w.profile_id == profile_id)
            .cloned()
            .ok_or_else(|| {
                NoctraError::WorkspaceError("No workspace found for profile".to_string())
            })?;

        *self.active_profile_id.write() = profile.id;
        *self.active_workspace_id.write() = ws.id;
        Ok((profile, ws))
    }

    pub fn list_workspaces_for_active_profile(&self) -> Vec<Workspace> {
        let pid = *self.active_profile_id.read();
        self.workspaces
            .read()
            .iter()
            .filter(|w| w.profile_id == pid)
            .cloned()
            .collect()
    }

    pub fn active_workspace(&self) -> NoctraResult<Workspace> {
        let wid = *self.active_workspace_id.read();
        self.workspaces
            .read()
            .iter()
            .find(|w| w.id == wid)
            .cloned()
            .ok_or_else(|| NoctraError::WorkspaceError("Active workspace not found".to_string()))
    }

    pub fn create_workspace(
        &self,
        name: String,
        icon: String,
        accent_color: String,
        session_mode: WorkspaceSessionMode,
        domain_routing_rules: Vec<String>,
    ) -> NoctraResult<Workspace> {
        let pid = *self.active_profile_id.read();
        let id = Uuid::new_v4();
        let storage_partition_id = match session_mode {
            WorkspaceSessionMode::SharedSession => format!("part_shared_{}", pid),
            WorkspaceSessionMode::Isolated => format!("part_isolated_{}_{}", pid, id),
        };

        let ws = Workspace {
            id,
            profile_id: pid,
            name,
            icon,
            accent_color,
            session_mode,
            storage_partition_id,
            startup_pages: vec!["https://www.google.com".to_string()],
            domain_routing_rules,
            proxy_override: None,
            layout_mode: WindowLayoutMode::FreeFloating,
            created_at: Utc::now(),
        };

        let mut guard = self.workspaces.write();
        guard.push(ws.clone());
        self.db.save_workspaces(&guard)?;
        Ok(ws)
    }

    pub fn switch_workspace(&self, workspace_id: Uuid) -> NoctraResult<Workspace> {
        let pid = *self.active_profile_id.read();
        let ws = self
            .workspaces
            .read()
            .iter()
            .find(|w| w.id == workspace_id && w.profile_id == pid)
            .cloned()
            .ok_or_else(|| {
                NoctraError::WorkspaceError(
                    "Workspace not found or belongs to a different profile".to_string(),
                )
            })?;
        *self.active_workspace_id.write() = ws.id;
        Ok(ws)
    }

    /// Automatic Domain-Based Routing (Section 8): checks if a URL domain matches a workspace rule
    /// within the active profile and returns the target workspace if matched.
    pub fn resolve_domain_routed_workspace(&self, target_url: &str) -> Option<Workspace> {
        let parsed = Url::parse(target_url).ok()?;
        let host = parsed.host_str()?.to_lowercase();
        let pid = *self.active_profile_id.read();

        self.workspaces
            .read()
            .iter()
            .find(|w| {
                w.profile_id == pid
                    && w.domain_routing_rules
                        .iter()
                        .any(|rule| host == rule.to_lowercase() || host.ends_with(&format!(".{}", rule.to_lowercase())))
            })
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_and_workspace_storage_isolation() {
        let temp_dir = std::env::temp_dir().join(format!("noctra_prof_test_{}", Uuid::new_v4()));
        let db = Arc::new(NoctraDatabase::open_in_memory().unwrap());
        let engine = ProfileAndWorkspaceEngine::initialize(temp_dir.clone(), db.clone()).unwrap();

        let profiles = engine.list_profiles();
        assert_eq!(profiles.len(), 5); // Personal, Work, Development, Testing, Guest

        let personal = &profiles[0];
        let work = &profiles[1];
        assert_ne!(personal.cookie_namespace, work.cookie_namespace);
        assert_ne!(personal.storage_partition_path, work.storage_partition_path);

        // Verify SharedSession vs Isolated workspace partitions within Personal profile
        let workspaces = engine.list_workspaces_for_active_profile();
        let shared_ws = workspaces
            .iter()
            .find(|w| w.session_mode == WorkspaceSessionMode::SharedSession)
            .unwrap();
        let isolated_ws = workspaces
            .iter()
            .find(|w| w.session_mode == WorkspaceSessionMode::Isolated)
            .unwrap();

        assert_ne!(
            shared_ws.storage_partition_id,
            isolated_ws.storage_partition_id
        );

        // Verify isolated KV storage in database
        db.set_partition_storage(
            personal.id,
            &shared_ws.storage_partition_id,
            "https://github.com",
            "session_cookie",
            "shared_auth_token_123",
        )
        .unwrap();

        // Isolated workspace in the SAME profile must NOT see SharedSession storage
        let leaked_in_isolated = db
            .get_partition_storage(
                personal.id,
                &isolated_ws.storage_partition_id,
                "https://github.com",
                "session_cookie",
            )
            .unwrap();
        assert!(leaked_in_isolated.is_none());

        // Work profile must NOT see Personal profile storage
        let leaked_in_work = db
            .get_partition_storage(
                work.id,
                &shared_ws.storage_partition_id,
                "https://github.com",
                "session_cookie",
            )
            .unwrap();
        assert!(leaked_in_work.is_none());

        // Verify Domain-Based Automatic Routing
        let routed = engine
            .resolve_domain_routed_workspace("https://kubernetes.io/docs/home/")
            .unwrap();
        assert_eq!(routed.name, "DevOps & SRE");

        let _ = fs::remove_dir_all(temp_dir);
    }
}
