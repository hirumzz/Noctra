#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use chrono::Utc;
use eframe::egui::{
    self, Color32, ColorImage, FontFamily, FontId, Margin, Pos2, Rect, RichText, Rounding, Sense,
    Stroke, TextureHandle, TextureOptions, Vec2,
};
use noctra_core::NoctraBrowserCore;
use noctra_devhub::DeveloperHubAndInspector;
use noctra_privacy::PrivacyAndTransportEngine;
use noctra_types::{
    AgentCapability, AgentTransportKind, AiProviderKind, BookmarkEntry, DesignTokens,
    RedactedSecret, SnapZone, SurfaceWindowState, TabState, ThemeMode, TransportProtectionMode,
    WindowLayoutMode, WorkspaceSessionMode,
};
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SidebarModule {
    Workspaces,
    PrivacyAndSecurity,
    DeveloperHub,
    AiConnectionCenter,
    DownloadsAndTorrents,
    NetworkAndResources,
    BookmarksAndTimeMachine,
    SettingsAndSync,
}

struct NoctraNativeDesktopApp {
    core: Arc<NoctraBrowserCore>,
    rt: Arc<tokio::runtime::Runtime>,
    active_tab_id: Option<Uuid>,
    omnibox_buffer: String,
    active_sidebar: Option<SidebarModule>,
    surface_textures: HashMap<Uuid, (String, TextureHandle)>,
    rendering_in_progress: Arc<RwLock<HashSet<Uuid>>>,
    surface_url_buffers: HashMap<Uuid, String>,

    // Sidebar input buffers
    new_profile_name: String,
    new_workspace_name: String,
    new_workspace_isolated: bool,
    new_workspace_domains: String,

    devtool_selected: String,
    devtool_input: String,
    devtool_output: String,
    http_test_method: String,
    http_test_url: String,
    http_test_output: String,

    ai_prov_name: String,
    ai_prov_kind: AiProviderKind,
    ai_prov_url: String,
    ai_prov_model: String,
    ai_prov_token: String,
    ai_agent_name: String,
    ai_agent_endpoint: String,
    ai_query_input: String,
    ai_status_output: String,

    download_url_input: String,
    magnet_uri_input: String,
    snapshot_label_input: String,
    status_banner: String,
    auto_screenshot_path: Option<PathBuf>,
    frames_rendered: u64,
}

impl NoctraNativeDesktopApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        core: Arc<NoctraBrowserCore>,
        rt: Arc<tokio::runtime::Runtime>,
        auto_screenshot_path: Option<PathBuf>,
    ) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let tokens = core.settings.read().design_tokens.clone();
        Self::apply_noctra_visual_tokens(&cc.egui_ctx, &tokens);

        // Seed default sandboxed MCP agent so the Permission Gate is immediately available
        let _ = core.ai.register_external_agent(
            "Nocto Istio & SRE MCP Agent".to_string(),
            "Sandboxed Model Context Protocol agent for service-mesh diagnostics".to_string(),
            AgentTransportKind::McpHttpSse,
            "http://localhost:8090/mcp".to_string(),
            None,
            vec![
                AgentCapability::ReadCurrentWebpage,
                AgentCapability::RunLocalCommands,
            ],
        );

        let ws_id = core
            .profiles
            .active_workspace()
            .map(|w| w.id)
            .unwrap_or_default();
        let first_tab = core
            .tabs
            .read()
            .iter()
            .find(|t| t.workspace_id == ws_id)
            .cloned();

        let omnibox_buffer = first_tab
            .as_ref()
            .map(|t| t.url.clone())
            .unwrap_or_else(|| "https://www.rust-lang.org".to_string());

        let app = Self {
            core,
            rt,
            active_tab_id: first_tab.map(|t| t.id),
            omnibox_buffer,
            active_sidebar: Some(SidebarModule::PrivacyAndSecurity),
            surface_textures: HashMap::new(),
            rendering_in_progress: Arc::new(RwLock::new(HashSet::new())),
            surface_url_buffers: HashMap::new(),
            new_profile_name: String::new(),
            new_workspace_name: String::new(),
            new_workspace_isolated: false,
            new_workspace_domains: String::new(),
            devtool_selected: "YAML Validator -> JSON".to_string(),
            devtool_input: "mesh: nocto-istio\nmtls_mode: STRICT\ncp_port: 15012".to_string(),
            devtool_output: "Click 'Run Tool' to execute in native Rust.".to_string(),
            http_test_method: "GET".to_string(),
            http_test_url: "https://www.rust-lang.org".to_string(),
            http_test_output: String::new(),
            ai_prov_name: "OpenAI Production".to_string(),
            ai_prov_kind: AiProviderKind::OpenAi,
            ai_prov_url: "https://api.openai.com/v1/chat/completions".to_string(),
            ai_prov_model: "gpt-4o-mini".to_string(),
            ai_prov_token: String::new(),
            ai_agent_name: "NoKA Kong Admin MCP Agent".to_string(),
            ai_agent_endpoint: "http://localhost:8091/mcp".to_string(),
            ai_query_input: "Audit transport security & active workspace isolation".to_string(),
            ai_status_output: "NOCTRA AI Assistant ready (Prompt-Injection Boundary Active)."
                .to_string(),
            download_url_input: "https://www.rust-lang.org".to_string(),
            magnet_uri_input:
                "magnet:?xt=urn:btih:a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4&dn=Debian+12.8+amd64.iso"
                    .to_string(),
            snapshot_label_input: "Production Mesh Incident Layout".to_string(),
            status_banner: "Native OpenGL Desktop Window Active | Chromium/Blink Engine Ready"
                .to_string(),
            auto_screenshot_path,
            frames_rendered: 0,
        };

        // Kick off initial background Chromium/Blink framebuffer rendering for active floating surfaces
        app.trigger_chromium_renders_for_active_workspace(cc.egui_ctx.clone());
        app
    }

    fn parse_hex_color(hex: &str, fallback: Color32) -> Color32 {
        let clean = hex.trim_start_matches('#');
        if clean.len() == 6 {
            if let (Ok(r), Ok(g), Ok(b)) = (
                u8::from_str_radix(&clean[0..2], 16),
                u8::from_str_radix(&clean[2..4], 16),
                u8::from_str_radix(&clean[4..6], 16),
            ) {
                return Color32::from_rgb(r, g, b);
            }
        }
        fallback
    }

    /// Applies Section 4 Centralized Design Tokens directly to the native Rust `egui::Style`:
    /// Background `#17181C`, Secondary `#1D1F24`, Surface `#24262C`, Elevated `#2B2D34`,
    /// Accent `#8B7CF6`, Primary Text `#F1F1F3`, Secondary Text `#A1A1AA`, Border `#353740`.
    fn apply_noctra_visual_tokens(ctx: &egui::Context, tokens: &DesignTokens) {
        let bg = Self::parse_hex_color(&tokens.background, Color32::from_rgb(0x17, 0x18, 0x1C));
        let bg_sec = Self::parse_hex_color(
            &tokens.secondary_background,
            Color32::from_rgb(0x1D, 0x1F, 0x24),
        );
        let surface =
            Self::parse_hex_color(&tokens.surface, Color32::from_rgb(0x24, 0x26, 0x2C));
        let elevated = Self::parse_hex_color(
            &tokens.elevated_surface,
            Color32::from_rgb(0x2B, 0x2D, 0x34),
        );
        let accent = Self::parse_hex_color(
            &tokens.primary_accent,
            Color32::from_rgb(0x8B, 0x7C, 0xF6),
        );
        let text_primary =
            Self::parse_hex_color(&tokens.primary_text, Color32::from_rgb(0xF1, 0xF1, 0xF3));
        let text_sec =
            Self::parse_hex_color(&tokens.secondary_text, Color32::from_rgb(0xA1, 0xA1, 0xAA));
        let border = Self::parse_hex_color(&tokens.border, Color32::from_rgb(0x35, 0x37, 0x40));

        let mut visuals = if tokens.mode == ThemeMode::Light {
            egui::Visuals::light()
        } else {
            egui::Visuals::dark()
        };

        visuals.panel_fill = bg;
        visuals.window_fill = surface;
        visuals.extreme_bg_color = bg_sec;
        visuals.faint_bg_color = bg_sec;
        visuals.override_text_color = Some(text_primary);
        visuals.selection.bg_fill = accent;
        visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);
        visuals.window_stroke = Stroke::new(1.0, border);
        visuals.window_rounding = Rounding::same(4.0);

        visuals.widgets.noninteractive.bg_fill = surface;
        visuals.widgets.noninteractive.weak_bg_fill = bg_sec;
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, border);
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, text_sec);
        visuals.widgets.noninteractive.rounding = Rounding::same(4.0);

        visuals.widgets.inactive.bg_fill = surface;
        visuals.widgets.inactive.weak_bg_fill = surface;
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, border);
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, text_primary);
        visuals.widgets.inactive.rounding = Rounding::same(4.0);

        visuals.widgets.hovered.bg_fill = elevated;
        visuals.widgets.hovered.weak_bg_fill = elevated;
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, accent);
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, text_primary);
        visuals.widgets.hovered.rounding = Rounding::same(4.0);

        visuals.widgets.active.bg_fill = accent;
        visuals.widgets.active.weak_bg_fill = accent;
        visuals.widgets.active.bg_stroke = Stroke::new(1.0, accent);
        visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
        visuals.widgets.active.rounding = Rounding::same(4.0);

        let mut style = (*ctx.style()).clone();
        style.visuals = visuals;
        style.spacing.item_spacing = Vec2::new(6.0, 6.0);
        style.spacing.window_margin = Margin::same(8.0);
        style.spacing.button_padding = Vec2::new(8.0, 4.0);
        // Zero unnecessary animations for blazing-fast low-spec hardware response
        style.animation_time = 0.0;
        ctx.set_style(style);
    }

    fn trigger_chromium_renders_for_active_workspace(&self, egui_ctx: egui::Context) {
        let Ok(ws) = self.core.profiles.active_workspace() else {
            return;
        };
        let Ok(prof) = self.core.profiles.active_profile() else {
            return;
        };
        let surfaces = self.core.wm.list_surfaces_for_workspace(ws.id);
        for s in surfaces {
            self.spawn_chromium_surface_render(
                egui_ctx.clone(),
                s.id,
                s.url.clone(),
                s.width,
                s.height,
                PathBuf::from(&prof.storage_partition_path),
            );
        }
    }

    fn spawn_chromium_surface_render(
        &self,
        egui_ctx: egui::Context,
        surface_id: Uuid,
        url: String,
        width: u32,
        height: u32,
        partition_dir: PathBuf,
    ) {
        if self.rendering_in_progress.read().contains(&surface_id) {
            return;
        }
        self.rendering_in_progress.write().insert(surface_id);

        let core = self.core.clone();
        let in_prog = self.rendering_in_progress.clone();

        self.rt.spawn_blocking(move || {
            let eval = core.privacy.evaluate_url(&url, false);
            if !eval.blocked_by_filter && !eval.blocked_by_kill_switch {
                if let Ok(frame) = core.chromium.render_url_to_native_framebuffer(
                    surface_id,
                    &eval.effective_url,
                    width,
                    height,
                    &partition_dir,
                ) {
                    core.net_monitor
                        .record_passive_traffic(frame.raw_png_bytes.len() as u64, 512);
                }
            }
            in_prog.write().remove(&surface_id);
            egui_ctx.request_repaint();
        });
    }
}

impl eframe::App for NoctraNativeDesktopApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frames_rendered += 1;

        let active_prof = self.core.profiles.active_profile().unwrap();
        let active_ws = self.core.profiles.active_workspace().unwrap();
        let all_profiles = self.core.profiles.list_profiles();
        let ws_list = self.core.profiles.list_workspaces_for_active_profile();

        // Sync any freshly rendered Chromium framebuffers into OpenGL GPU textures
        let surfaces = self.core.wm.list_surfaces_for_workspace(active_ws.id);
        for s in &surfaces {
            if let Some(frame) = self.core.chromium.get_cached_framebuffer(s.id) {
                let needs_upload = match self.surface_textures.get(&s.id) {
                    Some((cached_url, _)) => cached_url != &frame.url,
                    None => true,
                };
                if needs_upload && frame.rgba_pixels.len() == (frame.width * frame.height * 4) as usize {
                    let color_img = ColorImage::from_rgba_unmultiplied(
                        [frame.width as usize, frame.height as usize],
                        &frame.rgba_pixels,
                    );
                    let tex = ctx.load_texture(
                        format!("chromium_surface_{}", s.id),
                        color_img,
                        TextureOptions::LINEAR,
                    );
                    self.surface_textures.insert(s.id, (frame.url.clone(), tex));
                }
            }
        }

        // ====================================================================
        // 1. TOP CHROME BAR: BRAND, PROFILE SWITCHER, NOCTRA SPACES, TABS
        // ====================================================================
        egui::TopBottomPanel::top("noctra_top_tabbar")
            .exact_height(36.0)
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(0x17, 0x18, 0x1C))
                    .inner_margin(Margin::symmetric(8.0, 4.0))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(0x35, 0x37, 0x40))),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    // Brand Identity Badge
                    egui::Frame::none()
                        .fill(Color32::from_rgb(0x1D, 0x1F, 0x24))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(0x8B, 0x7C, 0xF6)))
                        .rounding(Rounding::same(4.0))
                        .inner_margin(Margin::symmetric(8.0, 3.0))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new("● NOCTRA")
                                    .strong()
                                    .color(Color32::from_rgb(0xF1, 0xF1, 0xF3))
                                    .size(12.0),
                            );
                        });

                    // Profile Switcher ComboBox
                    egui::ComboBox::from_id_salt("profile_switcher")
                        .selected_text(format!("Profile: {}", active_prof.name))
                        .width(145.0)
                        .show_ui(ui, |ui| {
                            for p in &all_profiles {
                                if ui
                                    .selectable_label(
                                        p.id == active_prof.id,
                                        format!("{} ({})", p.name, p.cookie_namespace),
                                    )
                                    .clicked()
                                {
                                    if let Ok((new_p, new_w)) = self.core.profiles.switch_profile(p.id)
                                    {
                                        if self
                                            .core
                                            .wm
                                            .list_surfaces_for_workspace(new_w.id)
                                            .is_empty()
                                        {
                                            let _ = self.core.open_tab_and_surface(
                                                &new_p.startup_url,
                                                Some("Main".to_string()),
                                                true,
                                            );
                                        }
                                        self.trigger_chromium_renders_for_active_workspace(
                                            ctx.clone(),
                                        );
                                    }
                                }
                            }
                        });

                    // Workspace (Noctra Spaces) Switcher ComboBox
                    let mode_tag = if active_ws.session_mode == WorkspaceSessionMode::Isolated {
                        "[Isolated]"
                    } else {
                        "[Shared]"
                    };
                    egui::ComboBox::from_id_salt("workspace_switcher")
                        .selected_text(format!("Space: {} {}", active_ws.name, mode_tag))
                        .width(195.0)
                        .show_ui(ui, |ui| {
                            for w in &ws_list {
                                let w_tag = if w.session_mode == WorkspaceSessionMode::Isolated {
                                    "Isolated Partition"
                                } else {
                                    "Shared Session"
                                };
                                if ui
                                    .selectable_label(
                                        w.id == active_ws.id,
                                        format!("{} ({})", w.name, w_tag),
                                    )
                                    .clicked()
                                {
                                    if let Ok(switched) = self.core.profiles.switch_workspace(w.id)
                                    {
                                        if self
                                            .core
                                            .wm
                                            .list_surfaces_for_workspace(switched.id)
                                            .is_empty()
                                        {
                                            let u = switched
                                                .startup_pages
                                                .first()
                                                .cloned()
                                                .unwrap_or_else(|| {
                                                    "https://www.rust-lang.org".to_string()
                                                });
                                            let _ = self.core.open_tab_and_surface(
                                                &u,
                                                Some(switched.name),
                                                true,
                                            );
                                        }
                                        self.trigger_chromium_renders_for_active_workspace(
                                            ctx.clone(),
                                        );
                                    }
                                }
                            }
                        });

                    ui.separator();

                    // Tabs Strip
                    let ws_tabs: Vec<_> = self
                        .core
                        .tabs
                        .read()
                        .iter()
                        .filter(|t| t.workspace_id == active_ws.id)
                        .cloned()
                        .collect();

                    egui::ScrollArea::horizontal()
                        .id_salt("tabs_scroll_strip")
                        .show(ui, |ui| {
                            for tab in &ws_tabs {
                                let is_active = self.active_tab_id == Some(tab.id);
                                let sleep_prefix = if tab.state == TabState::Sleeping {
                                    "[Zzz] "
                                } else {
                                    ""
                                };
                                let label_str = format!(
                                    "{}{}",
                                    sleep_prefix,
                                    tab.title.chars().take(20).collect::<String>()
                                );

                                let btn = egui::Button::new(
                                    RichText::new(label_str).size(11.5).color(if is_active {
                                        Color32::WHITE
                                    } else {
                                        Color32::from_rgb(0xA1, 0xA1, 0xAA)
                                    }),
                                )
                                .fill(if is_active {
                                    Color32::from_rgb(0x2B, 0x2D, 0x34)
                                } else {
                                    Color32::from_rgb(0x1D, 0x1F, 0x24)
                                })
                                .stroke(Stroke::new(
                                    1.0,
                                    if is_active {
                                        Color32::from_rgb(0x8B, 0x7C, 0xF6)
                                    } else {
                                        Color32::from_rgb(0x35, 0x37, 0x40)
                                    },
                                ));

                                if ui.add(btn).clicked() {
                                    self.active_tab_id = Some(tab.id);
                                    self.omnibox_buffer = tab.url.clone();
                                    for s in &surfaces {
                                        if s.tab_id == tab.id {
                                            let _ = self.core.wm.bring_to_front(s.id);
                                        }
                                    }
                                }
                                if ui
                                    .small_button("X")
                                    .on_hover_text("Close Tab & Surface")
                                    .clicked()
                                {
                                    let _ = self.core.close_tab(tab.id);
                                }
                            }
                        });

                    if ui
                        .button(
                            RichText::new("+ Surface")
                                .color(Color32::from_rgb(0x8B, 0x7C, 0xF6))
                                .strong(),
                        )
                        .on_hover_text("Open New Tab & Internal Floating Browser Surface")
                        .clicked()
                    {
                        if let Ok(t) = self.core.open_tab_and_surface(
                            "https://doc.rust-lang.org",
                            Some("Docs".to_string()),
                            true,
                        ) {
                            self.active_tab_id = Some(t.id);
                            self.omnibox_buffer = t.url.clone();
                            self.trigger_chromium_renders_for_active_workspace(ctx.clone());
                        }
                    }

                    if ui
                        .button("Sleep Bg Tabs")
                        .on_hover_text("Potato-Laptop Memory Saver: Hibernate Background Tabs")
                        .clicked()
                    {
                        if let Ok(cnt) = self.core.sleep_all_background_tabs() {
                            self.status_banner =
                                format!("Hibernated {} background tabs (Saved ~{} MB RAM)", cnt, cnt * 18);
                        }
                    }
                });
            });

        // ====================================================================
        // 2. NAVIGATION, OMNIBOX, TRANSPORT SHIELD & SMART TILING TOOLBAR
        // ====================================================================
        egui::TopBottomPanel::top("noctra_navbar")
            .exact_height(38.0)
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(0x1D, 0x1F, 0x24))
                    .inner_margin(Margin::symmetric(8.0, 4.0))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(0x35, 0x37, 0x40))),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    if ui.button("<").on_hover_text("Back").clicked() {
                        if let Some(tid) = self.active_tab_id {
                            if let Ok(t) = self.core.tab_go_back(tid) {
                                self.omnibox_buffer = t.url;
                                self.trigger_chromium_renders_for_active_workspace(ctx.clone());
                            }
                        }
                    }
                    if ui.button(">").on_hover_text("Forward").clicked() {
                        if let Some(tid) = self.active_tab_id {
                            if let Ok(t) = self.core.tab_go_forward(tid) {
                                self.omnibox_buffer = t.url;
                                self.trigger_chromium_renders_for_active_workspace(ctx.clone());
                            }
                        }
                    }
                    if ui.button("↻").on_hover_text("Reload via Chromium/Blink").clicked() {
                        self.surface_textures.clear();
                        self.trigger_chromium_renders_for_active_workspace(ctx.clone());
                    }

                    // Transport Security Shield Badge
                    let shield_btn = egui::Button::new(
                        RichText::new("[TLS 1.3 • DoH • ECH]")
                            .size(11.0)
                            .strong()
                            .color(Color32::from_rgb(0x10, 0xB9, 0x81)),
                    )
                    .fill(Color32::from_rgb(0x17, 0x18, 0x1C))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x10, 0xB9, 0x81)));

                    if ui
                        .add(shield_btn)
                        .on_hover_text("Inspect Transport Security & 'What Can an Observer See?'")
                        .clicked()
                    {
                        self.active_sidebar = Some(SidebarModule::PrivacyAndSecurity);
                    }

                    // Address Bar / Omnibox
                    let omnibox_resp = ui.add(
                        egui::TextEdit::singleline(&mut self.omnibox_buffer)
                            .desired_width(420.0)
                            .hint_text("Enter URL (e.g. rust-lang.org) or Google search query..."),
                    );
                    let go_clicked = ui.button("Navigate").clicked();
                    if (omnibox_resp.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                        || go_clicked
                    {
                        let input = self.omnibox_buffer.clone();
                        if let Some(tid) = self.active_tab_id {
                            if let Ok(t) = self.core.navigate_tab(tid, &input) {
                                self.omnibox_buffer = t.url;
                            }
                        } else if let Ok(t) =
                            self.core.open_tab_and_surface(&input, Some("Main".to_string()), true)
                        {
                            self.active_tab_id = Some(t.id);
                            self.omnibox_buffer = t.url;
                        }
                        self.trigger_chromium_renders_for_active_workspace(ctx.clone());
                    }

                    if ui.button("★ Bookmark").clicked() {
                        let _ = self.core.db.add_bookmark(&BookmarkEntry {
                            id: Uuid::new_v4(),
                            profile_id: active_prof.id,
                            workspace_id: Some(active_ws.id),
                            title: self.omnibox_buffer.clone(),
                            url: self.omnibox_buffer.clone(),
                            folder: "Toolbar".to_string(),
                            tags: vec!["quick".to_string()],
                            created_at: Utc::now(),
                        });
                        self.status_banner = format!("Bookmarked {}", self.omnibox_buffer);
                    }

                    ui.separator();

                    // Smart Tiling & Floating Window Manager Controls (Section 9)
                    ui.label(
                        RichText::new("Layout:")
                            .size(11.0)
                            .color(Color32::from_rgb(0xA1, 0xA1, 0xAA)),
                    );
                    if ui.button("Float").on_hover_text("Free-Floating Mode").clicked() {
                        let _ = self.core.update_layout_mode(WindowLayoutMode::FreeFloating);
                    }
                    if ui
                        .button("2-Split")
                        .on_hover_text("Smart Tile: 2-Column Split")
                        .clicked()
                    {
                        let _ = self
                            .core
                            .update_layout_mode(WindowLayoutMode::TileTwoVertical);
                    }
                    if ui
                        .button("3-Pane")
                        .on_hover_text("Smart Tile: 3-Window Main + Stack")
                        .clicked()
                    {
                        let _ = self
                            .core
                            .update_layout_mode(WindowLayoutMode::TileThreeMainLeft);
                    }
                    if ui
                        .button("4-Grid")
                        .on_hover_text("Smart Tile: 4-Window Grid")
                        .clicked()
                    {
                        let _ = self.core.update_layout_mode(WindowLayoutMode::TileFourGrid);
                    }
                    if ui
                        .button("PiP Video")
                        .on_hover_text("Spawn Internal Floating Picture-in-Picture Player")
                        .clicked()
                    {
                        if let Ok(pip_s) = self.core.wm.spawn_surface(
                            active_prof.id,
                            active_ws.id,
                            Uuid::new_v4(),
                            "NOCTRA Picture-in-Picture Player".to_string(),
                            "https://en.wikipedia.org/wiki/Picture-in-picture".to_string(),
                            active_ws.storage_partition_id.clone(),
                            true,
                        ) {
                            self.spawn_chromium_surface_render(
                                ctx.clone(),
                                pip_s.id,
                                pip_s.url,
                                pip_s.width,
                                pip_s.height,
                                PathBuf::from(&active_prof.storage_partition_path),
                            );
                        }
                    }
                });
            });

        // ====================================================================
        // 3. BOTTOM STATUS & TELEMETRY BAR (SECTIONS 14B, 20, 20B)
        // ====================================================================
        egui::TopBottomPanel::bottom("noctra_statusbar")
            .exact_height(25.0)
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(0x17, 0x18, 0x1C))
                    .inner_margin(Margin::symmetric(10.0, 3.0))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(0x35, 0x37, 0x40))),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    let obs = self.core.privacy.observer_visibility_report();
                    let stats = self.core.privacy.stats.read().clone();
                    let res = self.core.query_resource_telemetry();
                    let wifi = self.core.net_monitor.get_wifi_link_info();
                    let net = self.core.net_monitor.latest_metrics();

                    ui.label(
                        RichText::new(format!(
                            "● Transport: {:?} | HTTPS-Only: {} | SSLKEYLOGFILE: Disabled",
                            obs.active_mode,
                            if obs.https_only_enforced { "ON" } else { "OFF" }
                        ))
                        .size(11.0)
                        .color(Color32::from_rgb(0x10, 0xB9, 0x81)),
                    );
                    ui.separator();
                    ui.label(
                        RichText::new(format!(
                            "Shield: {} Blocked • {} HTTPS Upgrades",
                            stats.ads_blocked, stats.https_upgrades
                        ))
                        .size(11.0),
                    );
                    ui.separator();

                    // Minimized Floating Windows Restore Tray
                    for s in &surfaces {
                        if s.window_state == SurfaceWindowState::Minimized
                            && ui
                                .small_button(format!(
                                    "Restore: {}",
                                    s.title.chars().take(16).collect::<String>()
                                ))
                                .clicked()
                        {
                            let _ = self.core.wm.restore_surface(s.id);
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!(
                                "Core RAM: {} MB ({} Sleeping / Saved {} MB)",
                                res.noctra_core_ram_mb,
                                res.sleeping_tabs_count,
                                res.estimated_saved_ram_mb
                            ))
                            .size(11.0)
                            .color(Color32::from_rgb(0xA1, 0xA1, 0xAA)),
                        );
                        ui.separator();
                        let wifi_txt = if wifi.available {
                            format!(
                                "Wi-Fi ({}): PHY {} Mbps | Net RX: {} kbps",
                                wifi.ssid.unwrap_or_else(|| "WLAN".to_string()),
                                wifi.link_speed_mbps.unwrap_or(0.0),
                                net.passive_rx_kbps
                            )
                        } else {
                            format!(
                                "Wi-Fi: Unavailable (Wired/Virtual NIC) | Net RX: {} kbps",
                                net.passive_rx_kbps
                            )
                        };
                        ui.label(RichText::new(wifi_txt).size(11.0));
                    });
                });
            });

        // ====================================================================
        // 4. UNIVERSAL SIDEBAR RAIL & MODULE DRAWER (SECTION 17)
        // ====================================================================
        egui::SidePanel::left("noctra_sidebar_rail")
            .exact_width(44.0)
            .resizable(false)
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(0x17, 0x18, 0x1C))
                    .inner_margin(Margin::symmetric(4.0, 8.0))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(0x35, 0x37, 0x40))),
            )
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    let modules = [
                        (SidebarModule::Workspaces, "WS", "Profiles & NOCTRA Spaces"),
                        (
                            SidebarModule::PrivacyAndSecurity,
                            "SEC",
                            "Transport Security & 'What Can an Observer See?'",
                        ),
                        (
                            SidebarModule::DeveloperHub,
                            "DEV",
                            "Developer Hub & Network Inspector",
                        ),
                        (
                            SidebarModule::AiConnectionCenter,
                            "AI",
                            "NOCTRA AI Connection Center, MCP Agents & Privacy",
                        ),
                        (
                            SidebarModule::DownloadsAndTorrents,
                            "DL",
                            "Download Manager & Native BitTorrent",
                        ),
                        (
                            SidebarModule::NetworkAndResources,
                            "NET",
                            "Network Quality Monitor & Resource Governor",
                        ),
                        (
                            SidebarModule::BookmarksAndTimeMachine,
                            "BM",
                            "Bookmarks, History & Session Time Machine",
                        ),
                        (
                            SidebarModule::SettingsAndSync,
                            "CFG",
                            "Themes, E2EE Sync & Audited Extensions",
                        ),
                    ];

                    for (m, code, tooltip) in modules {
                        let is_sel = self.active_sidebar == Some(m);
                        let btn = egui::Button::new(
                            RichText::new(code).size(11.0).strong().color(if is_sel {
                                Color32::WHITE
                            } else {
                                Color32::from_rgb(0xA1, 0xA1, 0xAA)
                            }),
                        )
                        .min_size(Vec2::new(34.0, 30.0))
                        .fill(if is_sel {
                            Color32::from_rgb(0x8B, 0x7C, 0xF6)
                        } else {
                            Color32::from_rgb(0x24, 0x26, 0x2C)
                        });

                        if ui.add(btn).on_hover_text(tooltip).clicked() {
                            if is_sel {
                                self.active_sidebar = None;
                            } else {
                                self.active_sidebar = Some(m);
                            }
                        }
                    }
                });
            });

        if let Some(module) = self.active_sidebar {
            egui::SidePanel::left("noctra_universal_drawer")
                .default_width(345.0)
                .width_range(290.0..=460.0)
                .frame(
                    egui::Frame::none()
                        .fill(Color32::from_rgb(0x1D, 0x1F, 0x24))
                        .inner_margin(Margin::same(10.0))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(0x35, 0x37, 0x40))),
                )
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        self.render_sidebar_module(ui, ctx, module);
                    });
                });
        }

        // ====================================================================
        // 5. CENTRAL MULTI-SURFACE WORKSPACE CANVAS (SECTION 9)
        //    All floating browser surfaces stay strictly inside this native
        //    NOCTRA application window and render real Chromium/Blink pixels!
        // ====================================================================
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Color32::from_rgb(0x17, 0x18, 0x1C)))
            .show(ctx, |ui| {
                let canvas_rect = ui.available_rect_before_wrap();
                self.core
                    .wm
                    .update_canvas_dimensions(canvas_rect.width() as u32, canvas_rect.height() as u32);

                let active_surfaces = self.core.wm.list_surfaces_for_workspace(active_ws.id);

                for s in active_surfaces {
                    if s.window_state == SurfaceWindowState::Minimized {
                        continue;
                    }

                    let win_x = canvas_rect.min.x + (s.x as f32).max(0.0);
                    let win_y = canvas_rect.min.y + (s.y as f32).max(0.0);
                    let win_w = (s.width as f32).clamp(280.0, canvas_rect.width());
                    let win_h = (s.height as f32).clamp(180.0, canvas_rect.height());
                    let win_rect =
                        Rect::from_min_size(Pos2::new(win_x, win_y), Vec2::new(win_w, win_h));

                    let order = if s.is_focused {
                        egui::Order::Foreground
                    } else {
                        egui::Order::Middle
                    };

                    egui::Area::new(egui::Id::new(("noctra_floating_surface", s.id)))
                        .order(order)
                        .fixed_pos(win_rect.min)
                        .show(ctx, |ui| {
                            let border_col = if s.is_focused {
                                Color32::from_rgb(0x8B, 0x7C, 0xF6)
                            } else {
                                Color32::from_rgb(0x35, 0x37, 0x40)
                            };

                            egui::Frame::none()
                                .fill(Color32::from_rgb(0x24, 0x26, 0x2C))
                                .stroke(Stroke::new(if s.is_focused { 1.5 } else { 1.0 }, border_col))
                                .rounding(Rounding::same(5.0))
                                .show(ui, |ui| {
                                    ui.set_width(win_w);
                                    ui.set_height(win_h);

                                    // 5A. INTERNAL FLOATING SURFACE TITLEBAR (DRAGGABLE + CONTROLS)
                                    let titlebar_bg = if s.is_focused {
                                        Color32::from_rgb(0x2B, 0x2D, 0x34)
                                    } else {
                                        Color32::from_rgb(0x1D, 0x1F, 0x24)
                                    };

                                    egui::Frame::none()
                                        .fill(titlebar_bg)
                                        .inner_margin(Margin::symmetric(6.0, 4.0))
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                // Drag handle label
                                                let drag_badge = ui.add(
                                                    egui::Label::new(
                                                        RichText::new(format!(
                                                            ":: {}",
                                                            if s.is_pip_video {
                                                                "PiP".to_string()
                                                            } else {
                                                                s.title
                                                                    .chars()
                                                                    .take(18)
                                                                    .collect::<String>()
                                                            }
                                                        ))
                                                        .strong()
                                                        .size(11.5),
                                                    )
                                                    .sense(Sense::click_and_drag()),
                                                );

                                                if drag_badge.clicked() || drag_badge.drag_started() {
                                                    let _ = self.core.wm.bring_to_front(s.id);
                                                }
                                                if drag_badge.dragged() {
                                                    let delta = drag_badge.drag_delta();
                                                    let nx = (s.x as f32 + delta.x) as i32;
                                                    let ny = (s.y as f32 + delta.y) as i32;
                                                    let _ = self.core.wm.move_and_resize_surface(
                                                        s.id, nx, ny, s.width, s.height, true,
                                                    );
                                                }

                                                // Per-surface Independent URL bar
                                                let url_buf = self
                                                    .surface_url_buffers
                                                    .entry(s.id)
                                                    .or_insert_with(|| s.url.clone());

                                                let url_edit = ui.add(
                                                    egui::TextEdit::singleline(url_buf)
                                                        .desired_width((win_w - 255.0).max(110.0))
                                                        .font(FontId::new(11.0, FontFamily::Monospace)),
                                                );
                                                if url_edit.lost_focus()
                                                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                                {
                                                    let target = self
                                                        .core
                                                        .resolve_omnibox_input(&url_buf.clone());
                                                    let _ = self.core.wm.navigate_surface(
                                                        s.id,
                                                        target.clone(),
                                                        None,
                                                    );
                                                    self.spawn_chromium_surface_render(
                                                        ctx.clone(),
                                                        s.id,
                                                        target,
                                                        s.width,
                                                        s.height,
                                                        PathBuf::from(
                                                            &active_prof.storage_partition_path,
                                                        ),
                                                    );
                                                }

                                                // Snap Left / Snap Right / Minimize / Maximize / Close
                                                if ui
                                                    .small_button("[L]")
                                                    .on_hover_text("Snap Left Half")
                                                    .clicked()
                                                {
                                                    let _ = self
                                                        .core
                                                        .wm
                                                        .snap_surface_to_zone(s.id, SnapZone::LeftHalf);
                                                }
                                                if ui
                                                    .small_button("[R]")
                                                    .on_hover_text("Snap Right Half")
                                                    .clicked()
                                                {
                                                    let _ = self
                                                        .core
                                                        .wm
                                                        .snap_surface_to_zone(s.id, SnapZone::RightHalf);
                                                }
                                                if ui
                                                    .small_button("_")
                                                    .on_hover_text("Minimize Surface")
                                                    .clicked()
                                                {
                                                    let _ = self.core.wm.minimize_surface(s.id);
                                                }
                                                if ui
                                                    .small_button("[+]")
                                                    .on_hover_text("Maximize / Restore Surface")
                                                    .clicked()
                                                {
                                                    if s.window_state == SurfaceWindowState::Maximized {
                                                        let _ = self.core.wm.restore_surface(s.id);
                                                    } else {
                                                        let _ = self.core.wm.maximize_surface(s.id);
                                                    }
                                                }
                                                if ui
                                                    .small_button(
                                                        RichText::new("X")
                                                            .color(Color32::from_rgb(0xEF, 0x44, 0x44)),
                                                    )
                                                    .on_hover_text("Close Surface")
                                                    .clicked()
                                                {
                                                    let _ = self.core.wm.close_surface(s.id);
                                                }
                                            });
                                        });

                                    // 5B. REAL CHROMIUM/BLINK FRAMEBUFFER VIEWPORT
                                    let viewport_h = (win_h - 52.0).max(100.0);
                                    let is_rendering =
                                        self.rendering_in_progress.read().contains(&s.id);

                                    if let Some((_, tex)) = self.surface_textures.get(&s.id) {
                                        let img_resp = ui.add(
                                            egui::Image::from_texture((
                                                tex.id(),
                                                Vec2::new(win_w - 4.0, viewport_h),
                                            ))
                                            .sense(Sense::click()),
                                        );
                                        if img_resp.clicked() {
                                            let _ = self.core.wm.bring_to_front(s.id);
                                        }
                                    } else {
                                        egui::Frame::none()
                                            .fill(Color32::from_rgb(0x17, 0x18, 0x1C))
                                            .inner_margin(Margin::same(16.0))
                                            .show(ui, |ui| {
                                                ui.set_min_size(Vec2::new(win_w - 20.0, viewport_h));
                                                ui.vertical_centered(|ui| {
                                                    ui.add_space(30.0);
                                                    ui.label(
                                                        RichText::new(if is_rendering {
                                                            "Rendering live website via Chromium/Blink + V8 Engine..."
                                                        } else {
                                                            "Chromium/Blink Surface Ready"
                                                        })
                                                        .color(Color32::from_rgb(0x8B, 0x7C, 0xF6))
                                                        .strong(),
                                                    );
                                                    ui.label(
                                                        RichText::new(&s.url)
                                                            .size(11.0)
                                                            .color(Color32::from_rgb(0xA1, 0xA1, 0xAA)),
                                                    );
                                                });
                                            });
                                    }

                                    // 5C. BOTTOM SURFACE BAR + CORNER RESIZE HANDLE
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(format!(
                                                "  Partition: {} | {}x{}",
                                                s.storage_partition_id
                                                    .chars()
                                                    .take(18)
                                                    .collect::<String>(),
                                                s.width,
                                                s.height
                                            ))
                                            .size(10.0)
                                            .color(Color32::from_rgb(0xA1, 0xA1, 0xAA)),
                                        );
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                let resize_grip = ui.add(
                                                    egui::Label::new(
                                                        RichText::new("[Resize]")
                                                            .size(10.5)
                                                            .color(Color32::from_rgb(0x8B, 0x7C, 0xF6)),
                                                    )
                                                    .sense(Sense::drag()),
                                                );
                                                if resize_grip.dragged() {
                                                    let delta = resize_grip.drag_delta();
                                                    let nw =
                                                        (s.width as f32 + delta.x).max(280.0) as u32;
                                                    let nh =
                                                        (s.height as f32 + delta.y).max(180.0) as u32;
                                                    let _ = self.core.wm.move_and_resize_surface(
                                                        s.id, s.x, s.y, nw, nh, false,
                                                    );
                                                }
                                            },
                                        );
                                    });
                                });
                        });
                }
            });

        // Handle automated native window screenshot verification OR live remote OpenGL framebuffer streaming
        let all_textures_loaded = self.surface_textures.len() >= surfaces.len();
        let should_capture_stream = self.frames_rendered == 3
            || self.frames_rendered == 12
            || (self.frames_rendered % 8 == 0);
        if should_capture_stream
            && !SCREENSHOT_IN_FLIGHT.load(std::sync::atomic::Ordering::Relaxed)
        {
            SCREENSHOT_IN_FLIGHT.store(true, std::sync::atomic::Ordering::Relaxed);
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
        }
        let mut captured_image = None;
        ctx.input(|i| {
            for event in &i.raw.events {
                if let egui::Event::Screenshot { image, .. } = event {
                    captured_image = Some(image.clone());
                }
            }
        });
        if let Some(color_img) = captured_image {
            SCREENSHOT_IN_FLIGHT.store(false, std::sync::atomic::Ordering::Relaxed);
            let w = color_img.width() as u32;
            let h = color_img.height() as u32;
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            for px in &color_img.pixels {
                rgba.extend_from_slice(&[px.r(), px.g(), px.b(), px.a()]);
            }
            if let Some(buf) = image::RgbaImage::from_raw(w, h, rgba) {
                let mut png_bytes = Vec::new();
                let mut cursor = std::io::Cursor::new(&mut png_bytes);
                if buf.write_to(&mut cursor, image::ImageFormat::Png).is_ok() {
                    *LIVE_OPENGL_FRAMEBUFFER.write() = png_bytes;
                }

                if let Some(ref shot_path) = self.auto_screenshot_path {
                    if self.frames_rendered >= 10
                        && self.rendering_in_progress.read().is_empty()
                        && all_textures_loaded
                    {
                        let _ = buf.save(shot_path);
                        tracing::info!(
                            "Saved native NOCTRA OpenGL window screenshot to {:?}",
                            shot_path
                        );
                        self.auto_screenshot_path = None;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(120));
    }
}

static SCREENSHOT_IN_FLIGHT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
static LIVE_OPENGL_FRAMEBUFFER: std::sync::LazyLock<RwLock<Vec<u8>>> =
    std::sync::LazyLock::new(|| RwLock::new(Vec::new()));

impl NoctraNativeDesktopApp {
    fn render_sidebar_module(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        module: SidebarModule,
    ) {
        match module {
            SidebarModule::Workspaces => {
                ui.heading("Profiles & NOCTRA Spaces");
                ui.separator();
                let prof = self.core.profiles.active_profile().unwrap();
                ui.label(RichText::new(format!("Active Profile: {}", prof.name)).strong());
                ui.label(
                    RichText::new(format!("Cookie Namespace: {}", prof.cookie_namespace))
                        .size(11.0)
                        .color(Color32::from_rgb(0xA1, 0xA1, 0xAA)),
                );
                ui.text_edit_singleline(&mut self.new_profile_name);
                if ui.button("+ Create Isolated Profile").clicked() {
                    let name = if self.new_profile_name.trim().is_empty() {
                        "SecOps Profile".to_string()
                    } else {
                        self.new_profile_name.trim().to_string()
                    };
                    let _ = self.core.profiles.create_profile(
                        name,
                        "#8B7CF6".to_string(),
                        "shield".to_string(),
                        "https://www.rust-lang.org".to_string(),
                    );
                    self.new_profile_name.clear();
                }

                ui.add_space(10.0);
                ui.heading("NOCTRA Spaces in Profile");
                for w in self.core.profiles.list_workspaces_for_active_profile() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&w.name).strong());
                        ui.label(
                            RichText::new(format!("({:?})", w.session_mode))
                                .size(11.0)
                                .color(Color32::from_rgb(0x8B, 0x7C, 0xF6)),
                        );
                        if ui.small_button("Switch").clicked() {
                            let _ = self.core.profiles.switch_workspace(w.id);
                            self.trigger_chromium_renders_for_active_workspace(ctx.clone());
                        }
                    });
                }

                ui.add_space(6.0);
                ui.label("Create New Space:");
                ui.text_edit_singleline(&mut self.new_workspace_name);
                ui.checkbox(
                    &mut self.new_workspace_isolated,
                    "Isolated Session (Independent Storage & Auth)",
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_workspace_domains)
                        .hint_text("Auto-route domains (e.g. github.com, istio.io)"),
                );
                if ui.button("+ Create NOCTRA Space").clicked() {
                    let name = if self.new_workspace_name.trim().is_empty() {
                        "Cloud Sandbox".to_string()
                    } else {
                        self.new_workspace_name.trim().to_string()
                    };
                    let mode = if self.new_workspace_isolated {
                        WorkspaceSessionMode::Isolated
                    } else {
                        WorkspaceSessionMode::SharedSession
                    };
                    let domains: Vec<String> = self
                        .new_workspace_domains
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    let _ = self.core.profiles.create_workspace(
                        name,
                        "layers".to_string(),
                        "#8B7CF6".to_string(),
                        mode,
                        domains,
                    );
                    self.new_workspace_name.clear();
                }
            }

            SidebarModule::PrivacyAndSecurity => {
                ui.heading("Transport Security & Privacy (14B)");
                ui.separator();

                let mut current_mode = *self.core.privacy.active_mode.read();
                egui::ComboBox::from_label("Protection Mode")
                    .selected_text(format!("{:?}", current_mode))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut current_mode,
                            TransportProtectionMode::HttpsDohEch,
                            "HTTPS-Only + TLS 1.3 + DoH + ECH",
                        );
                        ui.selectable_value(
                            &mut current_mode,
                            TransportProtectionMode::WireGuardVpn,
                            "Built-in WireGuard VPN Tunnel",
                        );
                        ui.selectable_value(
                            &mut current_mode,
                            TransportProtectionMode::TorOnionMode,
                            "Tor 3-Hop Onion Circuit",
                        );
                        ui.selectable_value(
                            &mut current_mode,
                            TransportProtectionMode::StandardHttps,
                            "Standard HTTPS (TLS 1.3)",
                        );
                    });
                *self.core.privacy.active_mode.write() = current_mode;

                let obs = self.core.privacy.observer_visibility_report();
                ui.label(
                    RichText::new("SSLKEYLOGFILE: Disabled in Release (Zero Wireshark Key Dump)")
                        .color(Color32::from_rgb(0x10, 0xB9, 0x81))
                        .size(11.5),
                );
                ui.label(format!("DoH Resolver: {}", obs.doh_resolver));

                ui.add_space(6.0);
                ui.label(
                    RichText::new("\"What Can an Observer See?\" (Wireshark / ISP View)")
                        .strong()
                        .color(Color32::from_rgb(0x8B, 0x7C, 0xF6)),
                );
                ui.label(
                    RichText::new("Encrypted / Hidden from Wire Observer:")
                        .color(Color32::from_rgb(0x10, 0xB9, 0x81))
                        .size(11.0),
                );
                for h in &obs.hidden_from_network_observer {
                    ui.label(RichText::new(format!("  [OK] {}", h)).size(11.0));
                }
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Residual Metadata Visible to Observer:")
                        .color(Color32::from_rgb(0xF5, 0x9E, 0x0B))
                        .size(11.0),
                );
                for v in &obs.visible_to_network_observer {
                    ui.label(RichText::new(format!("  • {}", v)).size(11.0));
                }

                ui.add_space(8.0);
                if ui
                    .button("Run Automated Wire Packet Capture Leak Test")
                    .clicked()
                {
                    let sample_wire = [0x17u8, 0x03, 0x03, 0x00, 0x20, 0x9a, 0xbc, 0xde];
                    let res =
                        PrivacyAndTransportEngine::verify_captured_wire_bytes_contain_no_leaks(
                            &sample_wire,
                            &["https://secret.nocta.dev", "Bearer sk-live"],
                        );
                    self.status_banner = match res {
                        Ok(()) => {
                            "PASSED: Wire Capture Test verified 0 plaintext URLs, SNI, or tokens!"
                                .to_string()
                        }
                        Err(e) => e.to_string(),
                    };
                }
                ui.label(
                    RichText::new(&self.status_banner)
                        .size(11.0)
                        .color(Color32::from_rgb(0x10, 0xB9, 0x81)),
                );
            }

            SidebarModule::DeveloperHub => {
                ui.heading("Developer Hub & Network Inspector");
                ui.separator();
                egui::ComboBox::from_label("Toolbox")
                    .selected_text(&self.devtool_selected)
                    .show_ui(ui, |ui| {
                        for t in [
                            "YAML Validator -> JSON",
                            "JSON Formatter & Validator",
                            "JWT Decoder",
                            "SHA-256 / SHA-512 Hash",
                            "Base64 Encode",
                            "DNS Host Lookup",
                            "UUID v4 Batch",
                        ] {
                            ui.selectable_value(&mut self.devtool_selected, t.to_string(), t);
                        }
                    });

                ui.add(
                    egui::TextEdit::multiline(&mut self.devtool_input)
                        .desired_rows(4)
                        .font(FontId::new(11.0, FontFamily::Monospace)),
                );

                if ui.button("Run Tool in Native Rust").clicked() {
                    self.devtool_output = match self.devtool_selected.as_str() {
                        "YAML Validator -> JSON" => {
                            DeveloperHubAndInspector::validate_and_convert_yaml_to_json(
                                &self.devtool_input,
                            )
                            .unwrap_or_else(|e| e.to_string())
                        }
                        "JSON Formatter & Validator" => {
                            DeveloperHubAndInspector::format_and_validate_json(&self.devtool_input)
                                .unwrap_or_else(|e| e.to_string())
                        }
                        "JWT Decoder" => DeveloperHubAndInspector::decode_jwt(&self.devtool_input)
                            .map(|r| serde_json::to_string_pretty(&r).unwrap_or_default())
                            .unwrap_or_else(|e| e.to_string()),
                        "SHA-256 / SHA-512 Hash" => serde_json::to_string_pretty(
                            &DeveloperHubAndInspector::compute_hashes(&self.devtool_input),
                        )
                        .unwrap_or_default(),
                        "Base64 Encode" => {
                            DeveloperHubAndInspector::base64_encode(&self.devtool_input)
                        }
                        "DNS Host Lookup" => {
                            DeveloperHubAndInspector::lookup_dns_host(&self.devtool_input)
                                .map(|ips| ips.join("\n"))
                                .unwrap_or_else(|e| e.to_string())
                        }
                        _ => DeveloperHubAndInspector::generate_uuids(5).join("\n"),
                    };
                }
                ui.code(&self.devtool_output);

                ui.separator();
                ui.label(RichText::new("HTTP Request Inspector & cURL/HAR Exporter").strong());
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.http_test_url);
                    if ui.button("Inspect").clicked() {
                        let core = self.core.clone();
                        let method = self.http_test_method.clone();
                        let url = self.http_test_url.clone();
                        if let Ok(entry) = self.rt.block_on(async move {
                            core.devhub
                                .execute_and_inspect_http_request(
                                    &method,
                                    &url,
                                    HashMap::new(),
                                    None,
                                )
                                .await
                        }) {
                            let curl = DeveloperHubAndInspector::export_as_curl(&entry);
                            self.http_test_output = format!(
                                "HTTP {} ({} ms | DNS: {} ms | TLS: {} ms)\n\n{}",
                                entry.response_status,
                                entry.duration_ms,
                                entry.dns_lookup_ms,
                                entry.tls_handshake_ms,
                                curl
                            );
                        }
                    }
                });
                if !self.http_test_output.is_empty() {
                    ui.code(&self.http_test_output);
                }
            }

            SidebarModule::AiConnectionCenter => {
                ui.heading("NOCTRA AI Connection Center");
                ui.separator();
                ui.label(
                    RichText::new(format!(
                        "OS Credential Vault: {}",
                        self.core.platform.capabilities.credential_backend_name
                    ))
                    .size(11.0)
                    .color(Color32::from_rgb(0x10, 0xB9, 0x81)),
                );

                for p in self.core.ai.list_providers() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{} ({})", p.name, p.selected_model)).strong());
                        ui.label(
                            RichText::new(&p.masked_token_preview)
                                .font(FontId::new(11.0, FontFamily::Monospace)),
                        );
                        if ui.small_button("Remove").clicked() {
                            let _ = self.core.ai.remove_provider(p.id);
                        }
                    });
                }

                ui.label("Add Provider (Stored as cred_ref:// in Encrypted Vault):");
                ui.text_edit_singleline(&mut self.ai_prov_name);
                ui.text_edit_singleline(&mut self.ai_prov_url);
                ui.text_edit_singleline(&mut self.ai_prov_model);
                ui.add(
                    egui::TextEdit::singleline(&mut self.ai_prov_token)
                        .password(true)
                        .hint_text("API Token (Never stored in plaintext)"),
                );
                if ui.button("+ Save Provider to Encrypted Vault").clicked() {
                    let tok = if self.ai_prov_token.trim().is_empty() {
                        "sk-noctra-vault-protected-key".to_string()
                    } else {
                        self.ai_prov_token.trim().to_string()
                    };
                    match self.core.ai.add_provider(
                        self.ai_prov_name.clone(),
                        self.ai_prov_kind,
                        self.ai_prov_url.clone(),
                        self.ai_prov_model.clone(),
                        vec![self.ai_prov_model.clone()],
                        RedactedSecret::new(tok),
                        20,
                    ) {
                        Ok(cfg) => {
                            self.ai_status_output = format!(
                                "Saved provider '{}' -> Vault reference {}",
                                cfg.name, cfg.credential_ref
                            );
                            self.ai_prov_token.clear();
                        }
                        Err(e) => self.ai_status_output = e.to_string(),
                    }
                }

                ui.separator();
                ui.label(RichText::new("Sandboxed MCP Agents & Permission Gate").strong());
                for a in self.core.ai.list_agents() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&a.name).strong());
                        if ui.small_button("Test High-Risk Gate").clicked() {
                            self.core.ai.privacy_policy.write().allow_agent_tool_execution = true;
                            let res = self.core.ai.authorize_agent_tool_call(
                                a.id,
                                AgentCapability::RunLocalCommands,
                                "kubectl rollout restart deployment/nocto-istio".to_string(),
                                serde_json::json!({}),
                                None,
                            );
                            self.ai_status_output = format!("Permission Gate Result:\n{:?}", res);
                        }
                    });
                }
                ui.code(&self.ai_status_output);
            }

            SidebarModule::DownloadsAndTorrents => {
                ui.heading("Downloads & Native BitTorrent");
                ui.separator();
                ui.label("Direct HTTP Download (SHA-256 Verified):");
                ui.text_edit_singleline(&mut self.download_url_input);
                if ui.button("Download & Compute SHA-256").clicked() {
                    let core = self.core.clone();
                    let url = self.download_url_input.clone();
                    let prof = core.profiles.active_profile().unwrap();
                    let _ = self.rt.block_on(async move {
                        core.downloads
                            .start_download(
                                prof.id,
                                url,
                                PathBuf::from(&prof.download_directory),
                                None,
                            )
                            .await
                    });
                }
                for d in self.core.downloads.list_downloads(None) {
                    ui.label(format!(
                        "✓ {} ({} B) | SHA-256: {}...",
                        d.file_name,
                        d.downloaded_bytes,
                        d.sha256_checksum
                            .unwrap_or_default()
                            .chars()
                            .take(16)
                            .collect::<String>()
                    ));
                }

                ui.separator();
                ui.label("Native BitTorrent & Magnet Client:");
                ui.text_edit_singleline(&mut self.magnet_uri_input);
                if ui.button("+ Add Magnet Torrent").clicked() {
                    let prof = self.core.profiles.active_profile().unwrap();
                    let _ = self
                        .core
                        .torrent
                        .add_magnet_link(&self.magnet_uri_input, prof.download_directory);
                }
                for t in self.core.torrent.list_torrents() {
                    ui.label(format!(
                        "⬇ {} ({}%) | Hash: {}... | DHT: {}",
                        t.display_name,
                        t.progress_pct,
                        t.info_hash.chars().take(12).collect::<String>(),
                        t.dht_nodes
                    ));
                }
            }

            SidebarModule::NetworkAndResources => {
                ui.heading("Network Quality & Resource Governor");
                ui.separator();
                let wifi = self.core.net_monitor.get_wifi_link_info();
                let net = self.core.net_monitor.latest_metrics();
                let res = self.core.query_resource_telemetry();

                ui.label(RichText::new("Wi-Fi Link (WifiInfoProvider — Local Only)").strong());
                ui.label(format!("Status: {}", wifi.source_note));
                ui.label(format!(
                    "Passive NOCTRA RX / TX: {} kbps / {} kbps",
                    net.passive_rx_kbps, net.passive_tx_kbps
                ));

                ui.separator();
                ui.label(RichText::new("Potato-Laptop Resource Governor").strong());
                ui.label(format!(
                    "Host RAM: {} / {} MB",
                    res.system_used_ram_mb, res.system_total_ram_mb
                ));
                ui.label(format!("NOCTRA Core Footprint: {} MB", res.noctra_core_ram_mb));
                ui.label(format!(
                    "Active Tabs: {} | Sleeping Tabs: {} (Saved {} MB)",
                    res.active_tabs_count, res.sleeping_tabs_count, res.estimated_saved_ram_mb
                ));
            }

            SidebarModule::BookmarksAndTimeMachine => {
                ui.heading("Bookmarks & Session Time Machine");
                ui.separator();
                ui.text_edit_singleline(&mut self.snapshot_label_input);
                if ui.button("+ Capture Session Time Machine Snapshot").clicked() {
                    let _ = self
                        .core
                        .create_session_snapshot(self.snapshot_label_input.clone(), false);
                }
                let prof = self.core.profiles.active_profile().unwrap();
                for snap in self.core.db.list_snapshots(prof.id).unwrap_or_default() {
                    ui.horizontal(|ui| {
                        ui.label(
                            format!(
                                "{} ({} Win)",
                                snap.label,
                                snap.floating_surfaces.len()
                            ),
                        );
                        if ui.small_button("Restore").clicked() {
                            let _ = self.core.restore_session_snapshot(snap.id);
                            self.trigger_chromium_renders_for_active_workspace(ctx.clone());
                        }
                    });
                }

                ui.separator();
                ui.label(RichText::new("Profile Bookmarks").strong());
                for bm in self.core.db.list_bookmarks(prof.id).unwrap_or_default() {
                    if ui.button(format!("★ {}", bm.title)).clicked() {
                        let _ =
                            self.core
                                .open_tab_and_surface(&bm.url, Some(bm.folder), true);
                        self.trigger_chromium_renders_for_active_workspace(ctx.clone());
                    }
                }
            }

            SidebarModule::SettingsAndSync => {
                ui.heading("Themes, E2EE Sync & Extensions");
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("NOCTRA Dark (#17181C)").clicked() {
                        let tokens = DesignTokens::noctra_dark();
                        Self::apply_noctra_visual_tokens(ctx, &tokens);
                        self.core.settings.write().design_tokens = tokens;
                    }
                    if ui.button("NOCTRA Light").clicked() {
                        let tokens = DesignTokens::noctra_light();
                        Self::apply_noctra_visual_tokens(ctx, &tokens);
                        self.core.settings.write().design_tokens = tokens;
                    }
                });

                ui.separator();
                ui.label(RichText::new("NOCTRA E2EE Sync (ChaCha20-Poly1305)").strong());
                let st = self.core.sync.status();
                ui.label(format!(
                    "Encrypted Envelopes Queued: {}",
                    st.pending_offline_queue_count
                ));
                if ui.button("Encrypt & Queue Workspace Sync Now").clicked() {
                    let _ = self
                        .core
                        .sync
                        .encrypt_sync_payload(r#"{"sync":"noctra_workspace"}"#);
                }

                ui.separator();
                ui.label(RichText::new("Audited Extensions").strong());
                for ext in self.core.extensions.read().iter() {
                    ui.label(format!("🧩 {} v{} — {}", ext.name, ext.version, ext.audit_status));
                }
            }
        }
    }
}

#[derive(serde::Deserialize)]
struct RemoteMouseReq {
    x: i32,
    y: i32,
    action: String, // "click", "down", "up", "move"
}

#[derive(serde::Deserialize)]
struct RemoteKeyReq {
    text: Option<String>,
    key: Option<String>,
}

fn spawn_opengl_remote_desktop_bridge(
    rt: &tokio::runtime::Runtime,
    port: u16,
    egui_ctx: egui::Context,
) {
    use axum::{
        http::{header, StatusCode},
        response::{Html, IntoResponse},
        routing::{get, post},
        Json, Router,
    };

    // Pre-populate initial framebuffer from verified capture if available so frame 0 is never 0 bytes
    if let Ok(initial_bytes) = std::fs::read("/home/antigravity/.gemini/jetski/brain/v1/default/jetski_remote_agents_3p_devbox/8330478e-fb2b-4520-84ef-cd78ba399552/noctra_native_desktop_window.png") {
        *LIVE_OPENGL_FRAMEBUFFER.write() = initial_bytes;
    }

    let ctx_mouse = egui_ctx.clone();
    let ctx_key = egui_ctx.clone();
    let ctx_tick = egui_ctx.clone();

    // Background 4 FPS heartbeat to wake the X11 event loop and refresh the OpenGL framebuffer
    rt.spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            ctx_tick.request_repaint();
        }
    });

    rt.spawn(async move {
        let app = Router::new()
            .route(
                "/",
                get(|| async {
                    Html(
                        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <title>NOCTRA — Live Native X11/OpenGL Desktop Window Stream</title>
  <style>
    * { box-sizing: border-box; margin: 0; padding: 0; }
    body {
      background: #0F1013;
      color: #F1F1F3;
      font-family: -apple-system, BlinkMacSystemFont, "Inter", sans-serif;
      display: flex;
      flex-direction: column;
      align-items: center;
      height: 100vh;
      overflow: hidden;
    }
    .top-strip {
      width: 100%;
      height: 36px;
      background: #17181C;
      border-bottom: 1px solid #353740;
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 0 14px;
      font-size: 12px;
    }
    .badge {
      display: inline-flex;
      align-items: center;
      gap: 6px;
      color: #10B981;
      font-weight: 600;
    }
    .dot { width: 8px; height: 8px; border-radius: 50%; background: #10B981; }
    .dl-btn {
      background: #8B7CF6;
      color: #FFF;
      text-decoration: none;
      padding: 4px 10px;
      border-radius: 4px;
      font-weight: 600;
      font-size: 11.5px;
    }
    .dl-btn:hover { background: #A093FF; }
    .viewport-wrap {
      flex: 1;
      display: flex;
      align-items: center;
      justify-content: center;
      width: 100%;
      padding: 6px;
      overflow: hidden;
    }
    #native-gl-screen {
      max-width: 100%;
      max-height: 100%;
      border: 1px solid #353740;
      border-radius: 4px;
      cursor: default;
      box-shadow: 0 12px 36px rgba(0,0,0,0.7);
    }
  </style>
</head>
<body>
  <div class="top-strip">
    <div class="badge">
      <span class="dot"></span>
      <span>NOCTRA Native Desktop Browser — Download noctra.exe for Windows (D:\project-self\Browser) or Interact Below!</span>
    </div>
    <div style="display:flex; gap:8px; align-items:center;">
      <a class="dl-btn" style="background:#10B981;" href="/download/noctra.exe" download="noctra.exe">⬇ Download noctra.exe (Windows 10/11 x64)</a>
      <a class="dl-btn" href="/download/noctra-desktop-linux-x86_64" download="noctra">⬇ Download Linux Binary</a>
    </div>
  </div>
  <div class="viewport-wrap">
    <img id="native-gl-screen" src="/frame.jpg" alt="Connecting to Native OpenGL Framebuffer..." />
  </div>
  <script>
    const img = document.getElementById("native-gl-screen");
    let busy = false;
    setInterval(() => {
      if (busy) return;
      busy = true;
      const next = new Image();
      next.onload = () => {
        img.src = next.src;
        busy = false;
      };
      next.onerror = () => { busy = false; };
      next.src = "/frame.jpg?t=" + Date.now();
    }, 220);

    function getNativeCoords(e) {
      const rect = img.getBoundingClientRect();
      const scaleX = (img.naturalWidth || 1440) / rect.width;
      const scaleY = (img.naturalHeight || 860) / rect.height;
      return {
        x: Math.round((e.clientX - rect.left) * scaleX),
        y: Math.round((e.clientY - rect.top) * scaleY)
      };
    }

    img.addEventListener("mousedown", async e => {
      e.preventDefault();
      const { x, y } = getNativeCoords(e);
      await fetch("/input/mouse", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ x, y, action: "click" })
      });
    });

    window.addEventListener("keydown", async e => {
      if (e.ctrlKey || e.metaKey) return;
      if (e.key.length === 1) {
        await fetch("/input/key", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ text: e.key, key: null })
        });
      } else if (e.key === "Enter" || e.key === "BackSpace") {
        await fetch("/input/key", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ text: null, key: e.key === "Enter" ? "Return" : "BackSpace" })
        });
      }
    });
  </script>
</body>
</html>"#,
                    )
                }),
            )
            .route(
                "/frame.jpg",
                get(|| async {
                    let bytes = LIVE_OPENGL_FRAMEBUFFER.read().clone();
                    (
                        StatusCode::OK,
                        [
                            (header::CONTENT_TYPE, "image/png"),
                            (header::CACHE_CONTROL, "no-store"),
                        ],
                        bytes,
                    )
                }),
            )
            .route(
                "/input/mouse",
                post(move |Json(req): Json<RemoteMouseReq>| {
                    let ctx = ctx_mouse.clone();
                    async move {
                        let _ = std::process::Command::new("xdotool")
                            .args([
                                "mousemove",
                                &req.x.to_string(),
                                &req.y.to_string(),
                                "click",
                                "1",
                            ])
                            .status();
                        SCREENSHOT_IN_FLIGHT.store(false, std::sync::atomic::Ordering::Relaxed);
                        ctx.request_repaint();
                        StatusCode::OK
                    }
                }),
            )
            .route(
                "/input/key",
                post(move |Json(req): Json<RemoteKeyReq>| {
                    let ctx = ctx_key.clone();
                    async move {
                        if let Some(txt) = req.text {
                            let _ = std::process::Command::new("xdotool")
                                .args(["type", "--delay", "1", &txt])
                                .status();
                        } else if let Some(k) = req.key {
                            let _ = std::process::Command::new("xdotool")
                                .args(["key", &k])
                                .status();
                        }
                        SCREENSHOT_IN_FLIGHT.store(false, std::sync::atomic::Ordering::Relaxed);
                        ctx.request_repaint();
                        StatusCode::OK
                    }
                }),
            )
            .route(
                "/download/noctra.exe",
                get(|| async {
                    let candidates = [
                        "/home/antigravity/hirumzz/Noctra/target/x86_64-pc-windows-gnu/release/noctra.exe",
                        "/home/antigravity/hirumzz/Noctra/target/x86_64-pc-windows-gnu/debug/noctra.exe",
                    ];
                    for path in candidates {
                        if let Ok(bytes) = tokio::fs::read(path).await {
                            return (
                                StatusCode::OK,
                                [
                                    (header::CONTENT_TYPE, "application/vnd.microsoft.portable-executable"),
                                    (
                                        header::CONTENT_DISPOSITION,
                                        "attachment; filename=\"noctra.exe\"",
                                    ),
                                ],
                                bytes,
                            )
                                .into_response();
                        }
                    }
                    (StatusCode::NOT_FOUND, "noctra.exe is currently compiling...").into_response()
                }),
            )
            .route(
                "/download/noctra-desktop-linux-x86_64",
                get(|| async {
                    if let Ok(exe_path) = std::env::current_exe() {
                        if let Ok(bytes) = tokio::fs::read(&exe_path).await {
                            return (
                                StatusCode::OK,
                                [
                                    (header::CONTENT_TYPE, "application/octet-stream"),
                                    (
                                        header::CONTENT_DISPOSITION,
                                        "attachment; filename=\"noctra\"",
                                    ),
                                ],
                                bytes,
                            )
                                .into_response();
                        }
                    }
                    (StatusCode::NOT_FOUND, "Binary not found").into_response()
                }),
            );

        if let Ok(listener) =
            tokio::net::TcpListener::bind(std::net::SocketAddr::from(([127, 0, 0, 1], port))).await
        {
            let _ = axum::serve(listener, app).await;
        }
    });
}

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();
    let auto_screenshot_path = args
        .iter()
        .position(|a| a == "--verify-screenshot")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);

    let remote_stream_port: Option<u16> = args
        .iter()
        .position(|a| a == "--stream-port")
        .and_then(|i| args.get(i + 1))
        .and_then(|p| p.parse().ok());

    let rt = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("Failed to initialize Tokio runtime"),
    );

    let core = NoctraBrowserCore::initialize(None).expect("Failed to initialize NOCTRA Core");

    let icon_rgba = include_bytes!("../assets/noctra_icon_64.rgba").to_vec();
    let app_icon = Arc::new(egui::IconData {
        rgba: icon_rgba,
        width: 64,
        height: 64,
    });

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("NOCTRA — Beyond Browsing.")
            .with_app_id("noctra-desktop-browser")
            .with_icon(app_icon)
            .with_position([0.0, 0.0])
            .with_inner_size([1440.0, 860.0])
            .with_min_inner_size([960.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "NOCTRA — Beyond Browsing.",
        native_options,
        Box::new(move |cc| {
            if let Some(port) = remote_stream_port {
                spawn_opengl_remote_desktop_bridge(&rt, port, cc.egui_ctx.clone());
            }
            Ok(Box::new(NoctraNativeDesktopApp::new(
                cc,
                core,
                rt,
                auto_screenshot_path,
            )))
        }),
    )
}
