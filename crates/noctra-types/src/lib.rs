use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use thiserror::Error;
use uuid::Uuid;

// ============================================================================
// 1. ERROR TYPES
// ============================================================================

#[derive(Debug, Error)]
pub enum NoctraError {
    #[error("Profile error: {0}")]
    ProfileError(String),
    #[error("Workspace error: {0}")]
    WorkspaceError(String),
    #[error("Window manager error: {0}")]
    WindowManagerError(String),
    #[error("Security & policy violation: {0}")]
    SecurityViolation(String),
    #[error("Credential storage error: {0}")]
    CredentialError(String),
    #[error("Database error: {0}")]
    DatabaseError(String),
    #[error("Network / transport error: {0}")]
    NetworkError(String),
    #[error("AI provider or agent error: {0}")]
    AiError(String),
    #[error("Permission denied for capability '{capability}': {reason}")]
    PermissionDenied { capability: String, reason: String },
    #[error("Action requires explicit user approval (Approval ID: {approval_id})")]
    ApprovalRequired { approval_id: Uuid, description: String },
    #[error("Invalid input: {0}")]
    ValidationError(String),
    #[error("Internal error: {0}")]
    Internal(String),
}

pub type NoctraResult<T> = Result<T, NoctraError>;

// ============================================================================
// 2. REDACTED SECRET (NEVER LOGGED OR SERIALIZED IN PLAINTEXT)
// ============================================================================

/// Wraps sensitive credentials in memory and guarantees Debug/Display/Serialize
/// never leak plaintext secrets into logs, crash dumps, or UI responses.
#[derive(Clone)]
pub struct RedactedSecret(String);

impl RedactedSecret {
    pub fn new(secret: impl Into<String>) -> Self {
        Self(secret.into())
    }

    /// Deliberately explicit accessor so callers cannot accidentally print it.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }

    pub fn masked_preview(&self) -> String {
        let len = self.0.chars().count();
        if len <= 6 {
            "••••••••".to_string()
        } else {
            let prefix: String = self.0.chars().take(3).collect();
            let suffix: String = self.0.chars().skip(len - 2).collect();
            format!("{}••••••{}", prefix, suffix)
        }
    }
}

impl fmt::Debug for RedactedSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RedactedSecret([REDACTED])")
    }
}

impl fmt::Display for RedactedSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[REDACTED]")
    }
}

impl Serialize for RedactedSecret {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.masked_preview())
    }
}

// ============================================================================
// 3. CENTRALIZED DESIGN TOKEN SYSTEM (SECTION 4)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    Dark,
    Light,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiDensity {
    Compact,
    Comfortable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignTokens {
    pub mode: ThemeMode,
    pub density: UiDensity,
    pub background: String,
    pub secondary_background: String,
    pub surface: String,
    pub elevated_surface: String,
    pub primary_accent: String,
    pub accent_hover: String,
    pub primary_text: String,
    pub secondary_text: String,
    pub border: String,
    pub danger: String,
    pub warning: String,
    pub success: String,
    pub contrast_multiplier: f32,
    pub low_spec_performance_mode: bool,
}

impl Default for DesignTokens {
    fn default() -> Self {
        Self::noctra_dark()
    }
}

impl DesignTokens {
    /// Official NOCTRA Dark-First Color Grading (Section 4 specification)
    pub fn noctra_dark() -> Self {
        Self {
            mode: ThemeMode::Dark,
            density: UiDensity::Compact,
            background: "#17181C".to_string(),
            secondary_background: "#1D1F24".to_string(),
            surface: "#24262C".to_string(),
            elevated_surface: "#2B2D34".to_string(),
            primary_accent: "#8B7CF6".to_string(),
            accent_hover: "#A093FF".to_string(),
            primary_text: "#F1F1F3".to_string(),
            secondary_text: "#A1A1AA".to_string(),
            border: "#353740".to_string(),
            danger: "#EF4444".to_string(),
            warning: "#F59E0B".to_string(),
            success: "#10B981".to_string(),
            contrast_multiplier: 1.0,
            low_spec_performance_mode: true, // Default enabled for maximum speed on low-spec hardware
        }
    }

    pub fn noctra_light() -> Self {
        Self {
            mode: ThemeMode::Light,
            density: UiDensity::Compact,
            background: "#F4F5F7".to_string(),
            secondary_background: "#EAECEF".to_string(),
            surface: "#FFFFFF".to_string(),
            elevated_surface: "#F9FAFB".to_string(),
            primary_accent: "#6D5BD0".to_string(),
            accent_hover: "#5A48BE".to_string(),
            primary_text: "#17181C".to_string(),
            secondary_text: "#52525B".to_string(),
            border: "#D4D6DE".to_string(),
            danger: "#DC2626".to_string(),
            warning: "#D97706".to_string(),
            success: "#059669".to_string(),
            contrast_multiplier: 1.0,
            low_spec_performance_mode: true,
        }
    }

    pub fn to_css_variables(&self) -> String {
        let chrome_height = match self.density {
            UiDensity::Compact => "34px",
            UiDensity::Comfortable => "42px",
        };
        format!(
            ":root {{
  --noctra-bg: {};
  --noctra-bg-secondary: {};
  --noctra-surface: {};
  --noctra-surface-elevated: {};
  --noctra-accent: {};
  --noctra-accent-hover: {};
  --noctra-text: {};
  --noctra-text-secondary: {};
  --noctra-border: {};
  --noctra-danger: {};
  --noctra-warning: {};
  --noctra-success: {};
  --noctra-chrome-height: {};
  --noctra-transition: {};
}}",
            self.background,
            self.secondary_background,
            self.surface,
            self.elevated_surface,
            self.primary_accent,
            self.accent_hover,
            self.primary_text,
            self.secondary_text,
            self.border,
            self.danger,
            self.warning,
            self.success,
            chrome_height,
            if self.low_spec_performance_mode { "0ms" } else { "110ms ease-out" }
        )
    }
}

// ============================================================================
// 4. PROFILES & WORKSPACES (SECTIONS 6 & 8)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfilePreset {
    Personal,
    Work,
    Development,
    Testing,
    Guest,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserProfile {
    pub id: Uuid,
    pub name: String,
    pub preset: ProfilePreset,
    pub icon: String,
    pub color: String,
    pub startup_url: String,
    pub default_search_engine: String,
    pub storage_partition_path: String,
    pub cookie_namespace: String,
    pub download_directory: String,
    pub proxy_url: Option<String>,
    pub wireguard_enabled: bool,
    pub tor_mode_enabled: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceSessionMode {
    /// Shares the parent profile's cookies and authentication session.
    SharedSession,
    /// Has independent website storage and authentication sessions.
    Isolated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub name: String,
    pub icon: String,
    pub accent_color: String,
    pub session_mode: WorkspaceSessionMode,
    pub storage_partition_id: String,
    pub startup_pages: Vec<String>,
    pub domain_routing_rules: Vec<String>,
    pub proxy_override: Option<String>,
    pub layout_mode: WindowLayoutMode,
    pub created_at: DateTime<Utc>,
}

// ============================================================================
// 5. TABS & INTERNAL FLOATING WINDOW MANAGER (SECTIONS 9, 10, 11)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabOrientation {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabState {
    Active,
    Background,
    Sleeping, // Memory Saver / Tab Hibernation for Potato Laptops
    Loading,
    Crashed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserTab {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub workspace_id: Uuid,
    pub group_name: Option<String>,
    pub title: String,
    pub url: String,
    pub favicon: Option<String>,
    pub pinned: bool,
    pub muted: bool,
    pub state: TabState,
    pub zoom_level: f32,
    pub estimated_ram_kb: u64,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub history_stack: Vec<String>,
    pub history_index: usize,
    pub last_active_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowLayoutMode {
    FreeFloating,
    TileTwoHorizontal,
    TileTwoVertical,
    TileThreeMainLeft,
    TileFourGrid,
    Hybrid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapZone {
    None,
    LeftHalf,
    RightHalf,
    TopHalf,
    BottomHalf,
    TopLeftQuarter,
    TopRightQuarter,
    BottomLeftQuarter,
    BottomRightQuarter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceWindowState {
    Normal,
    Minimized,
    Maximized,
}

/// Represents an internal floating browser surface inside the main NOCTRA window.
/// Never spawns an OS window (Section 9).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InternalFloatingSurface {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub workspace_id: Uuid,
    pub tab_id: Uuid,
    pub title: String,
    pub url: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub restore_rect: Option<(i32, i32, u32, u32)>,
    pub z_index: u32,
    pub window_state: SurfaceWindowState,
    pub is_focused: bool,
    pub is_loading: bool,
    pub is_pip_video: bool,
    pub snap_zone: SnapZone,
    pub storage_partition_id: String,
}

// ============================================================================
// 6. TRANSPORT SECURITY & PRIVACY ENGINE (SECTIONS 14 & 14B)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportProtectionMode {
    StandardHttps,
    HttpsDohEch,
    WireGuardVpn,
    TorOnionMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObserverVisibilityReport {
    pub active_mode: TransportProtectionMode,
    pub https_only_enforced: bool,
    pub tls_min_version: String,
    pub doh_resolver: String,
    pub ech_enabled: bool,
    pub kill_switch_armed: bool,
    pub sslkeylogfile_disabled: bool,
    pub visible_to_network_observer: Vec<String>,
    pub hidden_from_network_observer: Vec<String>,
    pub residual_risk_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertificateChainInspection {
    pub host: String,
    pub subject: String,
    pub issuer: String,
    pub root_ca_name: String,
    pub is_publicly_trusted_root: bool,
    pub possible_mitm_corporate_proxy: bool,
    pub tls_version: String,
    pub cipher_suite: String,
    pub ech_negotiated: bool,
    pub sha256_fingerprint: String,
    pub pinned_match: Option<bool>,
}

// ============================================================================
// 7. NETWORK QUALITY MONITOR (SECTION 20B)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WifiLinkInfo {
    pub available: bool,
    pub interface_name: Option<String>,
    pub ssid: Option<String>,
    pub bssid: Option<String>,
    pub band: Option<String>,
    pub channel: Option<u32>,
    pub rssi_dbm: Option<i32>,
    pub link_speed_mbps: Option<f64>,
    pub source_note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InternetQualityMetrics {
    pub latency_ms: f64,
    pub jitter_ms: f64,
    pub packet_loss_pct: f64,
    pub download_throughput_mbps: Option<f64>,
    pub upload_throughput_mbps: Option<f64>,
    pub passive_rx_kbps: f64,
    pub passive_tx_kbps: f64,
    pub measured_at: DateTime<Utc>,
}

// ============================================================================
// 8. AI CONNECTION CENTER & AGENT PERMISSIONS (SECTIONS 21–26)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiProviderKind {
    OpenAi,
    Anthropic,
    GoogleGemini,
    OpenAiCompatible,
    OllamaLocal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiProviderConfig {
    pub id: Uuid,
    pub name: String,
    pub kind: AiProviderKind,
    pub base_url: String,
    pub selected_model: String,
    pub available_models: Vec<String>,
    /// Strictly an opaque reference into OS Credential Storage (`cred_ref://noctra/ai/<uuid>`), NEVER plaintext!
    pub credential_ref: String,
    pub masked_token_preview: String,
    pub enabled: bool,
    pub timeout_secs: u64,
    pub max_retries: u32,
    pub rate_limit_rpm: u32,
    pub total_tokens_used: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCapability {
    ReadCurrentWebpage,
    ReadSelectedText,
    ReadOpenTabs,
    SearchBookmarks,
    SearchHistory,
    ReadDownloadMetadata,
    CreateBrowserTabs,
    NavigateBrowserTabs,
    ManageWorkspaces,
    AccessLocalFiles,
    ExecuteBrowserAutomation,
    SendNetworkRequests,
    RunLocalCommands,
    DeleteFiles,
    ModifyBrowserConfig,
    SubmitForms,
}

impl AgentCapability {
    /// Section 24: High-risk operations ALWAYS require explicit user confirmation.
    pub fn is_high_risk(&self) -> bool {
        matches!(
            self,
            Self::AccessLocalFiles
                | Self::ExecuteBrowserAutomation
                | Self::SendNetworkRequests
                | Self::RunLocalCommands
                | Self::DeleteFiles
                | Self::ModifyBrowserConfig
                | Self::SubmitForms
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTransportKind {
    McpStdio,
    McpHttpSse,
    HttpJsonRpc,
    WebSocketAgent,
    OpenAiCompatibleAgent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalAgentConfig {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub transport: AgentTransportKind,
    pub endpoint: String,
    pub credential_ref: Option<String>,
    pub timeout_secs: u64,
    pub enabled: bool,
    pub granted_capabilities: Vec<AgentCapability>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiPrivacyPolicy {
    pub allow_send_webpage_content: bool,
    pub allow_send_selected_text: bool,
    pub allow_multi_tab_analysis: bool,
    pub allow_history_access: bool,
    pub allow_local_file_access: bool,
    pub store_conversations_locally: bool,
    pub sync_conversations: bool,
    pub allow_agent_tool_execution: bool,
    pub require_per_request_confirmation: bool,
}

impl Default for AiPrivacyPolicy {
    fn default() -> Self {
        Self {
            allow_send_webpage_content: false,
            allow_send_selected_text: true,
            allow_multi_tab_analysis: false,
            allow_history_access: false,
            allow_local_file_access: false,
            store_conversations_locally: true,
            sync_conversations: false,
            allow_agent_tool_execution: false,
            require_per_request_confirmation: true,
        }
    }
}

// ============================================================================
// 9. SESSION TIME MACHINE & BOOKMARKS/HISTORY (SECTIONS 5 & 18)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkEntry {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub workspace_id: Option<Uuid>,
    pub title: String,
    pub url: String,
    pub folder: String,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub workspace_id: Uuid,
    pub title: String,
    pub url: String,
    pub visited_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub workspace_id: Option<Uuid>,
    pub label: String,
    pub is_automatic: bool,
    pub tabs: Vec<BrowserTab>,
    pub floating_surfaces: Vec<InternalFloatingSurface>,
    pub layout_mode: WindowLayoutMode,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversalSidebarConfig {
    pub collapsed: bool,
    pub width_px: u32,
    pub pinned_modules: Vec<String>,
    pub active_module: Option<String>,
}

impl Default for UniversalSidebarConfig {
    fn default() -> Self {
        Self {
            collapsed: false,
            width_px: 310,
            pinned_modules: vec![
                "workspaces".to_string(),
                "tabs".to_string(),
                "bookmarks".to_string(),
                "history".to_string(),
                "downloads".to_string(),
                "torrents".to_string(),
                "privacy".to_string(),
                "devhub".to_string(),
                "network".to_string(),
                "ai".to_string(),
                "timemachine".to_string(),
                "notes".to_string(),
            ],
            active_module: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserSettings {
    pub design_tokens: DesignTokens,
    pub tab_orientation: TabOrientation,
    pub auto_sleep_tabs_after_secs: u64,
    pub memory_saver_enabled: bool,
    pub transport_mode: TransportProtectionMode,
    pub https_only_mode: bool,
    pub doh_resolver_url: String,
    pub ech_enabled: bool,
    pub block_ads_and_trackers: bool,
    pub block_third_party_cookies: bool,
    pub webrtc_ip_leak_protection: bool,
    pub vpn_kill_switch: bool,
    pub opt_in_telemetry: bool,
    pub sidebar: UniversalSidebarConfig,
    pub ai_privacy: AiPrivacyPolicy,
    pub custom_search_engines: HashMap<String, String>,
}

impl Default for BrowserSettings {
    fn default() -> Self {
        let mut search_engines = HashMap::new();
        search_engines.insert(
            "Google".to_string(),
            "https://www.google.com/search?q=%s".to_string(),
        );
        search_engines.insert(
            "DuckDuckGo".to_string(),
            "https://duckduckgo.com/?q=%s".to_string(),
        );
        search_engines.insert(
            "Brave Search".to_string(),
            "https://search.brave.com/search?q=%s".to_string(),
        );
        Self {
            design_tokens: DesignTokens::noctra_dark(),
            tab_orientation: TabOrientation::Horizontal,
            auto_sleep_tabs_after_secs: 45, // Aggressive potato-laptop optimization
            memory_saver_enabled: true,
            transport_mode: TransportProtectionMode::HttpsDohEch,
            https_only_mode: true,
            doh_resolver_url: "https://cloudflare-dns.com/dns-query".to_string(),
            ech_enabled: true,
            block_ads_and_trackers: true,
            block_third_party_cookies: true,
            webrtc_ip_leak_protection: true,
            vpn_kill_switch: true,
            opt_in_telemetry: false,
            sidebar: UniversalSidebarConfig::default(),
            ai_privacy: AiPrivacyPolicy::default(),
            custom_search_engines: search_engines,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redacted_secret_never_leaks_in_debug_or_display() {
        let secret = RedactedSecret::new("sk-live-super-secret-token-99999");
        let debug_str = format!("{:?}", secret);
        let display_str = format!("{}", secret);
        let json_str = serde_json::to_string(&secret).unwrap();

        assert!(!debug_str.contains("super-secret"));
        assert!(!display_str.contains("super-secret"));
        assert!(!json_str.contains("super-secret"));
        assert_eq!(secret.expose_secret(), "sk-live-super-secret-token-99999");
    }

    #[test]
    fn test_design_tokens_match_spec() {
        let tokens = DesignTokens::noctra_dark();
        assert_eq!(tokens.background, "#17181C");
        assert_eq!(tokens.secondary_background, "#1D1F24");
        assert_eq!(tokens.surface, "#24262C");
        assert_eq!(tokens.elevated_surface, "#2B2D34");
        assert_eq!(tokens.primary_accent, "#8B7CF6");
        assert_eq!(tokens.accent_hover, "#A093FF");
        assert_eq!(tokens.primary_text, "#F1F1F3");
        assert_eq!(tokens.secondary_text, "#A1A1AA");
        assert_eq!(tokens.border, "#353740");
    }
}
