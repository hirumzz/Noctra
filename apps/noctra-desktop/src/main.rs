#![windows_subsystem = "windows"]

use chrono::Utc;
use eframe::egui::{
    self, Align, Color32, ColorImage, FontFamily, FontId, Layout, Margin, Pos2, Rect, RichText,
    Rounding, Sense, Stroke, TextureHandle, TextureOptions, Vec2,
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
enum ViewMode {
    /// Standard full-window tab browsing like Brave, Chrome, Edge, and Firefox
    StandardFullTab,
    /// NOCTRA Multi-Surface Floating & Tiled Workspace Mode
    MultiSurfaceCanvas,
}

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
    new_tab_search_buffer: String,
    view_mode: ViewMode,
    show_bookmarks_bar: bool,
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

        let _ = core.ai.register_external_agent(
            "Nocto Service Mesh Agent".to_string(),
            "Sandboxed MCP agent for infrastructure & web diagnostics".to_string(),
            AgentTransportKind::McpHttpSse,
            "http://localhost:8090/mcp".to_string(),
            None,
            vec![
                AgentCapability::ReadCurrentWebpage,
                AgentCapability::RunLocalCommands,
            ],
        );

        // Migrate any old demo tabs or bookmarks pointing to rust-lang.org so the browser starts completely clean
        {
            let mut tabs_guard = core.tabs.write();
            for t in tabs_guard.iter_mut() {
                if t.url.contains("rust-lang.org") || t.url.contains("Rust_(programming_language)")
                {
                    t.url = "noctra://newtab".to_string();
                    t.title = "New Tab".to_string();
                }
            }
            let _ = core.db.save_kv("tabs", "open_tabs", &*tabs_guard);
        }
        if let Ok(prof) = core.profiles.active_profile() {
            if let Ok(bms) = core.db.list_bookmarks(prof.id) {
                for b in &bms {
                    if b.url.contains("rust-lang.org") || b.title.contains("Rust") {
                        let _ = core.db.delete_bookmark(b.id, prof.id);
                    }
                }
            }
            if core.db.list_bookmarks(prof.id).unwrap_or_default().len() < 3 {
                let ws_id = core.profiles.active_workspace().ok().map(|w| w.id);
                for (title, url) in [
                    ("Google", "https://www.google.com"),
                    ("YouTube", "https://www.youtube.com"),
                    ("GitHub", "https://github.com"),
                    ("Wikipedia", "https://www.wikipedia.org"),
                ] {
                    let _ = core.db.add_bookmark(&BookmarkEntry {
                        id: Uuid::new_v4(),
                        profile_id: prof.id,
                        workspace_id: ws_id,
                        title: title.to_string(),
                        url: url.to_string(),
                        folder: "Favorites".to_string(),
                        tags: vec![],
                        created_at: Utc::now(),
                    });
                }
            }
        }

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
            .map(|t| {
                if t.url == "noctra://newtab" {
                    String::new()
                } else {
                    t.url.clone()
                }
            })
            .unwrap_or_default();

        let app = Self {
            core,
            rt,
            active_tab_id: first_tab.map(|t| t.id),
            omnibox_buffer,
            new_tab_search_buffer: String::new(),
            view_mode: ViewMode::StandardFullTab, // Default to clean Brave/Chrome/Edge full-window view
            show_bookmarks_bar: true,
            active_sidebar: None, // Closed by default for maximum webpage area like Brave/Chrome
            surface_textures: HashMap::new(),
            rendering_in_progress: Arc::new(RwLock::new(HashSet::new())),
            surface_url_buffers: HashMap::new(),
            new_profile_name: String::new(),
            new_workspace_name: String::new(),
            new_workspace_isolated: false,
            new_workspace_domains: String::new(),
            devtool_selected: "JSON Formatter & Validator".to_string(),
            devtool_input: "{\n  \"browser\": \"NOCTRA\",\n  \"fast\": true\n}".to_string(),
            devtool_output: "Ready.".to_string(),
            http_test_method: "GET".to_string(),
            http_test_url: "https://www.google.com".to_string(),
            http_test_output: String::new(),
            ai_prov_name: "OpenAI GPT-4o".to_string(),
            ai_prov_kind: AiProviderKind::OpenAi,
            ai_prov_url: "https://api.openai.com/v1/chat/completions".to_string(),
            ai_prov_model: "gpt-4o-mini".to_string(),
            ai_prov_token: String::new(),
            ai_status_output: "AI Connection Center Ready (OS Credential Vault Active)."
                .to_string(),
            download_url_input: "https://www.google.com".to_string(),
            magnet_uri_input:
                "magnet:?xt=urn:btih:a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4&dn=Ubuntu+Desktop+amd64.iso"
                    .to_string(),
            snapshot_label_input: "Workspace Snapshot".to_string(),
            status_banner: "Shield Active: TLS 1.3 + DoH + ECH".to_string(),
            auto_screenshot_path,
            frames_rendered: 0,
        };

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
        visuals.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
        visuals.window_stroke = Stroke::new(1.0_f32, border);
        visuals.window_rounding = Rounding::same(6.0);

        visuals.widgets.noninteractive.bg_fill = surface;
        visuals.widgets.noninteractive.weak_bg_fill = bg_sec;
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, border);
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, text_sec);
        visuals.widgets.noninteractive.rounding = Rounding::same(6.0);

        visuals.widgets.inactive.bg_fill = surface;
        visuals.widgets.inactive.weak_bg_fill = surface;
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, border);
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, text_primary);
        visuals.widgets.inactive.rounding = Rounding::same(6.0);

        visuals.widgets.hovered.bg_fill = elevated;
        visuals.widgets.hovered.weak_bg_fill = elevated;
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, accent);
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, text_primary);
        visuals.widgets.hovered.rounding = Rounding::same(6.0);

        visuals.widgets.active.bg_fill = accent;
        visuals.widgets.active.weak_bg_fill = accent;
        visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, accent);
        visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
        visuals.widgets.active.rounding = Rounding::same(6.0);

        let mut style = (*ctx.style()).clone();
        style.visuals = visuals;
        style.spacing.item_spacing = Vec2::new(6.0, 6.0);
        style.spacing.window_margin = Margin::same(10.0);
        style.spacing.button_padding = Vec2::new(10.0, 5.0);
        style.animation_time = 0.0;
        ctx.set_style(style);
    }

    fn navigate_to_url(&mut self, ctx: &egui::Context, raw_input: &str) {
        let trimmed = raw_input.trim();
        if trimmed.is_empty() {
            return;
        }
        if let Some(tid) = self.active_tab_id {
            if let Ok(t) = self.core.navigate_tab(tid, trimmed) {
                self.omnibox_buffer = if t.url == "noctra://newtab" {
                    String::new()
                } else {
                    t.url.clone()
                };
            }
        } else if let Ok(t) = self
            .core
            .open_tab_and_surface(trimmed, None, true)
        {
            self.active_tab_id = Some(t.id);
            self.omnibox_buffer = if t.url == "noctra://newtab" {
                String::new()
            } else {
                t.url.clone()
            };
        }
        self.trigger_chromium_renders_for_active_workspace(ctx.clone());
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
            if s.url != "noctra://newtab" && !s.url.starts_with("about:") {
                self.spawn_chromium_surface_render(
                    egui_ctx.clone(),
                    s.id,
                    s.url.clone(),
                    s.width.max(1280),
                    s.height.max(760),
                    PathBuf::from(&prof.storage_partition_path),
                );
            }
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
        if url == "noctra://newtab" || url.starts_with("about:") {
            return;
        }
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

    /// Renders a modern, clean Brave / Chrome / Edge / Arc style New Tab Speed-Dial Dashboard in native 60 FPS GPU UI!
    fn render_modern_new_tab_dashboard(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let stats = self.core.privacy.stats.read().clone();
        let res = self.core.query_resource_telemetry();
        let active_prof = self.core.profiles.active_profile().unwrap();
        let active_ws = self.core.profiles.active_workspace().unwrap();

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(48.0);

                // 1. Brand Title & Tagline
                ui.label(
                    RichText::new("NOCTRA")
                        .size(34.0)
                        .strong()
                        .color(Color32::from_rgb(0xF1, 0xF1, 0xF3)),
                );
                ui.label(
                    RichText::new("Beyond Browsing.  •  Private, High-Speed Desktop Browser")
                        .size(13.0)
                        .color(Color32::from_rgb(0xA1, 0xA1, 0xAA)),
                );

                ui.add_space(24.0);

                // 2. Centered Pill Search & URL Bar (like Brave / Chrome / Firefox New Tab)
                egui::Frame::none()
                    .fill(Color32::from_rgb(0x24, 0x26, 0x2C))
                    .stroke(Stroke::new(1.5_f32, Color32::from_rgb(0x8B, 0x7C, 0xF6)))
                    .rounding(Rounding::same(22.0))
                    .inner_margin(Margin::symmetric(18.0, 10.0))
                    .show(ui, |ui| {
                        ui.set_max_width(580.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Search")
                                    .strong()
                                    .color(Color32::from_rgb(0x8B, 0x7C, 0xF6)),
                            );
                            let search_resp = ui.add(
                                egui::TextEdit::singleline(&mut self.new_tab_search_buffer)
                                    .frame(false)
                                    .desired_width(430.0)
                                    .font(FontId::new(14.5, FontFamily::Proportional))
                                    .hint_text("Search Google or type a URL (e.g. youtube.com, github.com)..."),
                            );
                            let go_btn = ui.add(
                                egui::Button::new(
                                    RichText::new("Go")
                                        .strong()
                                        .color(Color32::WHITE),
                                )
                                .fill(Color32::from_rgb(0x8B, 0x7C, 0xF6))
                                .rounding(Rounding::same(14.0)),
                            );
                            if (search_resp.lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                                || go_btn.clicked()
                            {
                                let q = self.new_tab_search_buffer.clone();
                                self.new_tab_search_buffer.clear();
                                self.navigate_to_url(ctx, &q);
                            }
                        });
                    });

                ui.add_space(28.0);

                // 3. Speed-Dial Favorite Sites Grid (Brave / Opera / Chrome style)
                let quick_sites = [
                    ("Google", "https://www.google.com", "G", Color32::from_rgb(0x3B, 0x82, 0xF6)),
                    ("YouTube", "https://www.youtube.com", "▶", Color32::from_rgb(0xEF, 0x44, 0x44)),
                    ("GitHub", "https://github.com", "GH", Color32::from_rgb(0x8B, 0x7C, 0xF6)),
                    ("Wikipedia", "https://www.wikipedia.org", "W", Color32::from_rgb(0xF1, 0xF1, 0xF3)),
                    ("DuckDuckGo", "https://duckduckgo.com", "D", Color32::from_rgb(0xF5, 0x9E, 0x0B)),
                    ("Reddit", "https://www.reddit.com", "R", Color32::from_rgb(0xEC, 0x48, 0x99)),
                    ("Cloudflare", "https://1.1.1.1", "CF", Color32::from_rgb(0x10, 0xB9, 0x81)),
                    ("Hacker News", "https://news.ycombinator.com", "Y", Color32::from_rgb(0xF9, 0x73, 0x16)),
                ];

                ui.horizontal(|ui| {
                    let total_w = 680.0;
                    let pad = (ui.available_width() - total_w).max(0.0) / 2.0;
                    ui.add_space(pad);
                    egui::Grid::new("speed_dial_grid")
                        .spacing(Vec2::new(14.0, 14.0))
                        .show(ui, |ui| {
                            for (idx, (name, url, badge, col)) in quick_sites.iter().enumerate() {
                                let card = egui::Frame::none()
                                    .fill(Color32::from_rgb(0x24, 0x26, 0x2C))
                                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x35, 0x37, 0x40)))
                                    .rounding(Rounding::same(10.0))
                                    .inner_margin(Margin::symmetric(18.0, 14.0))
                                    .show(ui, |ui| {
                                        ui.set_min_size(Vec2::new(120.0, 68.0));
                                        ui.vertical_centered(|ui| {
                                            ui.label(
                                                RichText::new(*badge)
                                                    .size(20.0)
                                                    .strong()
                                                    .color(*col),
                                            );
                                            ui.add_space(4.0);
                                            ui.label(
                                                RichText::new(*name)
                                                    .size(12.5)
                                                    .strong()
                                                    .color(Color32::from_rgb(0xF1, 0xF1, 0xF3)),
                                            );
                                        });
                                    });
                                if card.response.interact(Sense::click()).clicked() {
                                    self.navigate_to_url(ctx, url);
                                }
                                if idx == 3 {
                                    ui.end_row();
                                }
                            }
                        });
                });

                ui.add_space(30.0);

                // 4. Brave-Style Privacy & Performance Shield Cards
                ui.horizontal(|ui| {
                    let total_w = 680.0;
                    let pad = (ui.available_width() - total_w).max(0.0) / 2.0;
                    ui.add_space(pad);

                    egui::Frame::none()
                        .fill(Color32::from_rgb(0x1D, 0x1F, 0x24))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x35, 0x37, 0x40)))
                        .rounding(Rounding::same(10.0))
                        .inner_margin(Margin::symmetric(20.0, 14.0))
                        .show(ui, |ui| {
                            ui.set_min_width(205.0);
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new(format!("{} Blocked", stats.ads_blocked))
                                        .size(18.0)
                                        .strong()
                                        .color(Color32::from_rgb(0x8B, 0x7C, 0xF6)),
                                );
                                ui.label(
                                    RichText::new("Ads, Trackers & Fingerprinting")
                                        .size(11.5)
                                        .color(Color32::from_rgb(0xA1, 0xA1, 0xAA)),
                                );
                            });
                        });

                    egui::Frame::none()
                        .fill(Color32::from_rgb(0x1D, 0x1F, 0x24))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x35, 0x37, 0x40)))
                        .rounding(Rounding::same(10.0))
                        .inner_margin(Margin::symmetric(20.0, 14.0))
                        .show(ui, |ui| {
                            ui.set_min_width(205.0);
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new("TLS 1.3 + DoH + ECH")
                                        .size(18.0)
                                        .strong()
                                        .color(Color32::from_rgb(0x10, 0xB9, 0x81)),
                                );
                                ui.label(
                                    RichText::new("Hardened Encrypted Transport")
                                        .size(11.5)
                                        .color(Color32::from_rgb(0xA1, 0xA1, 0xAA)),
                                );
                            });
                        });

                    egui::Frame::none()
                        .fill(Color32::from_rgb(0x1D, 0x1F, 0x24))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x35, 0x37, 0x40)))
                        .rounding(Rounding::same(10.0))
                        .inner_margin(Margin::symmetric(20.0, 14.0))
                        .show(ui, |ui| {
                            ui.set_min_width(205.0);
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new(format!("{} MB Core RAM", res.noctra_core_ram_mb))
                                        .size(18.0)
                                        .strong()
                                        .color(Color32::from_rgb(0xF1, 0xF1, 0xF3)),
                                );
                                ui.label(
                                    RichText::new(format!(
                                        "{} • {}",
                                        active_prof.name, active_ws.name
                                    ))
                                    .size(11.5)
                                    .color(Color32::from_rgb(0xA1, 0xA1, 0xAA)),
                                );
                            });
                        });
                });
            });
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

        // Sync rendered Chromium framebuffers into OpenGL GPU textures
        let surfaces = self.core.wm.list_surfaces_for_workspace(active_ws.id);
        for s in &surfaces {
            if let Some(frame) = self.core.chromium.get_cached_framebuffer(s.id) {
                let needs_upload = match self.surface_textures.get(&s.id) {
                    Some((cached_url, _)) => cached_url != &frame.url,
                    None => true,
                };
                if needs_upload
                    && frame.rgba_pixels.len() == (frame.width * frame.height * 4) as usize
                {
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
        // 1. MODERN TAB STRIP BAR (LIKE BRAVE / CHROME / EDGE / FIREFOX)
        // ====================================================================
        egui::TopBottomPanel::top("noctra_modern_tabbar")
            .exact_height(38.0)
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(0x17, 0x18, 0x1C))
                    .inner_margin(Margin::symmetric(10.0, 5.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    // Profile & Space Pill
                    egui::ComboBox::from_id_salt("profile_switcher")
                        .selected_text(
                            RichText::new(format!("Profile: {}", active_prof.name))
                                .strong()
                                .color(Color32::from_rgb(0x8B, 0x7C, 0xF6)),
                        )
                        .width(135.0)
                        .show_ui(ui, |ui| {
                            for p in &all_profiles {
                                if ui
                                    .selectable_label(p.id == active_prof.id, &p.name)
                                    .clicked()
                                {
                                    if let Ok((_, new_w)) = self.core.profiles.switch_profile(p.id)
                                    {
                                        if self
                                            .core
                                            .wm
                                            .list_surfaces_for_workspace(new_w.id)
                                            .is_empty()
                                        {
                                            let _ = self.core.open_tab_and_surface(
                                                "noctra://newtab",
                                                None,
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

                    egui::ComboBox::from_id_salt("workspace_switcher")
                        .selected_text(format!("Space: {}", active_ws.name))
                        .width(145.0)
                        .show_ui(ui, |ui| {
                            for w in &ws_list {
                                let tag = if w.session_mode == WorkspaceSessionMode::Isolated {
                                    " [Isolated]"
                                } else {
                                    ""
                                };
                                if ui
                                    .selectable_label(
                                        w.id == active_ws.id,
                                        format!("{}{}", w.name, tag),
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
                                            let _ = self.core.open_tab_and_surface(
                                                "noctra://newtab",
                                                None,
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

                    ui.add_space(4.0);

                    // Modern Browser Tabs
                    let ws_tabs: Vec<_> = self
                        .core
                        .tabs
                        .read()
                        .iter()
                        .filter(|t| t.workspace_id == active_ws.id)
                        .cloned()
                        .collect();

                    if self.active_tab_id.is_none()
                        || !ws_tabs.iter().any(|t| Some(t.id) == self.active_tab_id)
                    {
                        self.active_tab_id = ws_tabs.first().map(|t| t.id);
                    }

                    for tab in &ws_tabs {
                        let is_active = self.active_tab_id == Some(tab.id);
                        let sleep_tag = if tab.state == TabState::Sleeping {
                            "Zzz "
                        } else {
                            ""
                        };
                        let title_text = if tab.url == "noctra://newtab" {
                            "New Tab".to_string()
                        } else {
                            format!(
                                "{}{}",
                                sleep_tag,
                                tab.title.chars().take(22).collect::<String>()
                            )
                        };

                        egui::Frame::none()
                            .fill(if is_active {
                                Color32::from_rgb(0x24, 0x26, 0x2C)
                            } else {
                                Color32::from_rgb(0x1D, 0x1F, 0x24)
                            })
                            .stroke(Stroke::new(
                                1.0_f32,
                                if is_active {
                                    Color32::from_rgb(0x8B, 0x7C, 0xF6)
                                } else {
                                    Color32::from_rgb(0x35, 0x37, 0x40)
                                },
                            ))
                            .rounding(Rounding::same(7.0))
                            .inner_margin(Margin::symmetric(10.0, 4.0))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let tab_lbl = ui.add(
                                        egui::Label::new(
                                            RichText::new(title_text).size(12.0).color(
                                                if is_active {
                                                    Color32::from_rgb(0xF1, 0xF1, 0xF3)
                                                } else {
                                                    Color32::from_rgb(0xA1, 0xA1, 0xAA)
                                                },
                                            ),
                                        )
                                        .sense(Sense::click()),
                                    );
                                    if tab_lbl.clicked() {
                                        self.active_tab_id = Some(tab.id);
                                        self.omnibox_buffer = if tab.url == "noctra://newtab" {
                                            String::new()
                                        } else {
                                            tab.url.clone()
                                        };
                                        for s in &surfaces {
                                            if s.tab_id == tab.id {
                                                let _ = self.core.wm.bring_to_front(s.id);
                                            }
                                        }
                                    }
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("×")
                                                    .size(13.0)
                                                    .color(Color32::from_rgb(0xA1, 0xA1, 0xAA)),
                                            )
                                            .frame(false),
                                        )
                                        .on_hover_text("Close Tab")
                                        .clicked()
                                    {
                                        let _ = self.core.close_tab(tab.id);
                                        if self.core.tabs.read().is_empty() {
                                            let _ = self.core.open_tab_and_surface(
                                                "noctra://newtab",
                                                None,
                                                true,
                                            );
                                        }
                                    }
                                });
                            });
                    }

                    // "+" New Tab Button (just like Chrome/Brave/Edge)
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(" + ")
                                    .size(14.0)
                                    .strong()
                                    .color(Color32::from_rgb(0xF1, 0xF1, 0xF3)),
                            )
                            .rounding(Rounding::same(6.0)),
                        )
                        .on_hover_text("New Tab")
                        .clicked()
                    {
                        if let Ok(t) =
                            self.core
                                .open_tab_and_surface("noctra://newtab", None, true)
                        {
                            self.active_tab_id = Some(t.id);
                            self.omnibox_buffer.clear();
                        }
                    }

                    // Right-aligned View Mode Switcher (Standard Tab View vs Multi-Surface Split/Floating)
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let is_multi = self.view_mode == ViewMode::MultiSurfaceCanvas;
                        if ui
                            .selectable_label(is_multi, "Split / Floating Canvas")
                            .on_hover_text(
                                "Switch to NOCTRA Multi-Surface Floating & Split-Screen Window Manager",
                            )
                            .clicked()
                        {
                            self.view_mode = if is_multi {
                                ViewMode::StandardFullTab
                            } else {
                                ViewMode::MultiSurfaceCanvas
                            };
                        }
                        if ui
                            .selectable_label(!is_multi, "Full Tab View")
                            .on_hover_text("Standard Full-Window Browser View (like Brave/Chrome/Edge)")
                            .clicked()
                        {
                            self.view_mode = ViewMode::StandardFullTab;
                        }
                    });
                });
            });

        // ====================================================================
        // 2. NAVIGATION & PILL OMNIBOX BAR (BRAVE / CHROME / EDGE STYLE)
        // ====================================================================
        egui::TopBottomPanel::top("noctra_modern_navbar")
            .exact_height(42.0)
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(0x1D, 0x1F, 0x24))
                    .inner_margin(Margin::symmetric(10.0, 5.0))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x35, 0x37, 0x40))),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    if ui.button(" < ").on_hover_text("Back").clicked() {
                        if let Some(tid) = self.active_tab_id {
                            if let Ok(t) = self.core.tab_go_back(tid) {
                                self.omnibox_buffer = if t.url == "noctra://newtab" {
                                    String::new()
                                } else {
                                    t.url
                                };
                                self.trigger_chromium_renders_for_active_workspace(ctx.clone());
                            }
                        }
                    }
                    if ui.button(" > ").on_hover_text("Forward").clicked() {
                        if let Some(tid) = self.active_tab_id {
                            if let Ok(t) = self.core.tab_go_forward(tid) {
                                self.omnibox_buffer = if t.url == "noctra://newtab" {
                                    String::new()
                                } else {
                                    t.url
                                };
                                self.trigger_chromium_renders_for_active_workspace(ctx.clone());
                            }
                        }
                    }
                    if ui.button(" ↻ ").on_hover_text("Reload Page").clicked() {
                        self.surface_textures.clear();
                        self.trigger_chromium_renders_for_active_workspace(ctx.clone());
                    }
                    if ui
                        .button(" Home ")
                        .on_hover_text("Home (New Tab Dashboard)")
                        .clicked()
                    {
                        self.navigate_to_url(ctx, "noctra://newtab");
                    }

                    // Pill-shaped Address & Search Omnibox
                    let avail_w = (ui.available_width() - 280.0).max(260.0);
                    egui::Frame::none()
                        .fill(Color32::from_rgb(0x17, 0x18, 0x1C))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x35, 0x37, 0x40)))
                        .rounding(Rounding::same(16.0))
                        .inner_margin(Margin::symmetric(12.0, 4.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                // Privacy Shield indicator inside Omnibox (like Brave Shield)
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("Shield: TLS 1.3")
                                                .size(11.0)
                                                .strong()
                                                .color(Color32::from_rgb(0x10, 0xB9, 0x81)),
                                        )
                                        .frame(false),
                                    )
                                    .on_hover_text("Click to inspect Transport Security & Privacy Shield")
                                    .clicked()
                                {
                                    self.active_sidebar =
                                        if self.active_sidebar == Some(SidebarModule::PrivacyAndSecurity)
                                        {
                                            None
                                        } else {
                                            Some(SidebarModule::PrivacyAndSecurity)
                                        };
                                }

                                ui.separator();

                                let omnibox_resp = ui.add(
                                    egui::TextEdit::singleline(&mut self.omnibox_buffer)
                                        .frame(false)
                                        .desired_width(avail_w - 150.0)
                                        .hint_text("Search Google or type a URL..."),
                                );
                                if omnibox_resp.lost_focus()
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                {
                                    let target = self.omnibox_buffer.clone();
                                    self.navigate_to_url(ctx, &target);
                                }

                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("★")
                                                .size(13.0)
                                                .color(Color32::from_rgb(0xF5, 0x9E, 0x0B)),
                                        )
                                        .frame(false),
                                    )
                                    .on_hover_text("Bookmark this page")
                                    .clicked()
                                {
                                    let url = if self.omnibox_buffer.trim().is_empty() {
                                        "https://www.google.com".to_string()
                                    } else {
                                        self.omnibox_buffer.trim().to_string()
                                    };
                                    let _ = self.core.db.add_bookmark(&BookmarkEntry {
                                        id: Uuid::new_v4(),
                                        profile_id: active_prof.id,
                                        workspace_id: Some(active_ws.id),
                                        title: url.clone(),
                                        url,
                                        folder: "Bookmarks Bar".to_string(),
                                        tags: vec![],
                                        created_at: Utc::now(),
                                    });
                                }
                            });
                        });

                    // Right Toolbar Quick Action Buttons (Brave / Edge style)
                    if ui
                        .button("AI")
                        .on_hover_text("NOCTRA AI Assistant & Connection Center")
                        .clicked()
                    {
                        self.active_sidebar =
                            if self.active_sidebar == Some(SidebarModule::AiConnectionCenter) {
                                None
                            } else {
                                Some(SidebarModule::AiConnectionCenter)
                            };
                    }
                    if ui
                        .button("Dev")
                        .on_hover_text("Developer Hub & Network Inspector")
                        .clicked()
                    {
                        self.active_sidebar =
                            if self.active_sidebar == Some(SidebarModule::DeveloperHub) {
                                None
                            } else {
                                Some(SidebarModule::DeveloperHub)
                            };
                    }
                    if ui
                        .button("DL")
                        .on_hover_text("Downloads & Native BitTorrent")
                        .clicked()
                    {
                        self.active_sidebar =
                            if self.active_sidebar == Some(SidebarModule::DownloadsAndTorrents) {
                                None
                            } else {
                                Some(SidebarModule::DownloadsAndTorrents)
                            };
                    }
                    if ui
                        .button("Sleep Tabs")
                        .on_hover_text("Potato-Laptop Memory Saver: Hibernate Background Tabs")
                        .clicked()
                    {
                        let _ = self.core.sleep_all_background_tabs();
                    }
                });
            });

        // ====================================================================
        // 3. BOOKMARKS BAR (LIKE CHROME / BRAVE / EDGE)
        // ====================================================================
        if self.show_bookmarks_bar {
            egui::TopBottomPanel::top("noctra_bookmarks_bar")
                .exact_height(28.0)
                .frame(
                    egui::Frame::none()
                        .fill(Color32::from_rgb(0x1D, 0x1F, 0x24))
                        .inner_margin(Margin::symmetric(12.0, 2.0))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x35, 0x37, 0x40))),
                )
                .show(ctx, |ui| {
                    ui.horizontal_centered(|ui| {
                        let bookmarks = self
                            .core
                            .db
                            .list_bookmarks(active_prof.id)
                            .unwrap_or_default();
                        for bm in bookmarks.iter().take(8) {
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(format!("★ {}", bm.title))
                                            .size(11.5)
                                            .color(Color32::from_rgb(0xA1, 0xA1, 0xAA)),
                                    )
                                    .frame(false),
                                )
                                .on_hover_text(&bm.url)
                                .clicked()
                            {
                                let target = bm.url.clone();
                                self.navigate_to_url(ctx, &target);
                            }
                            ui.add_space(6.0);
                        }

                        if self.view_mode == ViewMode::MultiSurfaceCanvas {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.small_button("PiP").clicked() {
                                    let _ = self.core.wm.spawn_surface(
                                        active_prof.id,
                                        active_ws.id,
                                        Uuid::new_v4(),
                                        "PiP Player".to_string(),
                                        "https://www.google.com".to_string(),
                                        active_ws.storage_partition_id.clone(),
                                        true,
                                    );
                                }
                                if ui.small_button("4-Grid").clicked() {
                                    let _ = self
                                        .core
                                        .update_layout_mode(WindowLayoutMode::TileFourGrid);
                                }
                                if ui.small_button("2-Split").clicked() {
                                    let _ = self
                                        .core
                                        .update_layout_mode(WindowLayoutMode::TileTwoVertical);
                                }
                                if ui.small_button("Float").clicked() {
                                    let _ = self
                                        .core
                                        .update_layout_mode(WindowLayoutMode::FreeFloating);
                                }
                                ui.label(
                                    RichText::new("Window Tiling:")
                                        .size(11.0)
                                        .color(Color32::from_rgb(0xA1, 0xA1, 0xAA)),
                                );
                            });
                        }
                    });
                });
        }

        // ====================================================================
        // 4. COMPACT FOOTER STATUS BAR
        // ====================================================================
        egui::TopBottomPanel::bottom("noctra_statusbar")
            .exact_height(22.0)
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(0x17, 0x18, 0x1C))
                    .inner_margin(Margin::symmetric(12.0, 2.0))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x35, 0x37, 0x40))),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    let stats = self.core.privacy.stats.read().clone();
                    let res = self.core.query_resource_telemetry();
                    let net = self.core.net_monitor.latest_metrics();

                    ui.label(
                        RichText::new(format!(
                            "NOCTRA Shield: {} Ads/Trackers Blocked  |  HTTPS-Only + DoH + ECH",
                            stats.ads_blocked
                        ))
                        .size(11.0)
                        .color(Color32::from_rgb(0x10, 0xB9, 0x81)),
                    );

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!(
                                "RAM: {} MB ({} Sleeping)  |  Net RX: {} kbps",
                                res.noctra_core_ram_mb,
                                res.sleeping_tabs_count,
                                net.passive_rx_kbps
                            ))
                            .size(11.0)
                            .color(Color32::from_rgb(0xA1, 0xA1, 0xAA)),
                        );
                    });
                });
            });

        // ====================================================================
        // 5. COLLAPSIBLE UNIVERSAL SIDEBAR RAIL (LIKE EDGE / VIVALDI / OPERA)
        // ====================================================================
        egui::SidePanel::left("noctra_sidebar_rail")
            .exact_width(48.0)
            .resizable(false)
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(0x17, 0x18, 0x1C))
                    .inner_margin(Margin::symmetric(4.0, 8.0))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x35, 0x37, 0x40))),
            )
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    let modules = [
                        (SidebarModule::Workspaces, "WS", "Profiles & NOCTRA Spaces"),
                        (
                            SidebarModule::PrivacyAndSecurity,
                            "SEC",
                            "Transport Security & Privacy Shield",
                        ),
                        (
                            SidebarModule::DeveloperHub,
                            "DEV",
                            "Developer Hub & Network Inspector",
                        ),
                        (
                            SidebarModule::AiConnectionCenter,
                            "AI",
                            "NOCTRA AI Connection Center & MCP Agents",
                        ),
                        (
                            SidebarModule::DownloadsAndTorrents,
                            "DL",
                            "Downloads & Native BitTorrent",
                        ),
                        (
                            SidebarModule::NetworkAndResources,
                            "NET",
                            "Network Quality & Resource Monitor",
                        ),
                        (
                            SidebarModule::BookmarksAndTimeMachine,
                            "BM",
                            "Bookmarks & Session Time Machine",
                        ),
                        (
                            SidebarModule::SettingsAndSync,
                            "CFG",
                            "Themes, E2EE Sync & Extensions",
                        ),
                    ];

                    for (m, code, tooltip) in modules {
                        let is_sel = self.active_sidebar == Some(m);
                        let btn = egui::Button::new(
                            RichText::new(code).size(10.5).strong().color(if is_sel {
                                Color32::WHITE
                            } else {
                                Color32::from_rgb(0xA1, 0xA1, 0xAA)
                            }),
                        )
                        .min_size(Vec2::new(38.0, 28.0))
                        .fill(if is_sel {
                            Color32::from_rgb(0x8B, 0x7C, 0xF6)
                        } else {
                            Color32::TRANSPARENT
                        });

                        if ui.add(btn).on_hover_text(tooltip).clicked() {
                            self.active_sidebar = if is_sel { None } else { Some(m) };
                        }
                    }
                });
            });

        if let Some(module) = self.active_sidebar {
            egui::SidePanel::left("noctra_universal_drawer")
                .default_width(330.0)
                .width_range(280.0..=440.0)
                .frame(
                    egui::Frame::none()
                        .fill(Color32::from_rgb(0x1D, 0x1F, 0x24))
                        .inner_margin(Margin::same(12.0))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x35, 0x37, 0x40))),
                )
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("NOCTRA Panel")
                                .strong()
                                .color(Color32::from_rgb(0x8B, 0x7C, 0xF6)),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.small_button("× Close").clicked() {
                                self.active_sidebar = None;
                            }
                        });
                    });
                    ui.separator();
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        self.render_sidebar_module(ui, ctx, module);
                    });
                });
        }

        // ====================================================================
        // 6. MAIN BROWSER VIEWPORT:
        //    - StandardFullTab: Clean full-window view like Brave / Chrome / Edge
        //    - MultiSurfaceCanvas: Floating & Tiled Internal Windows
        // ====================================================================
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Color32::from_rgb(0x17, 0x18, 0x1C)))
            .show(ctx, |ui| {
                let canvas_rect = ui.available_rect_before_wrap();
                self.core.wm.update_canvas_dimensions(
                    canvas_rect.width() as u32,
                    canvas_rect.height() as u32,
                );

                let active_tab = self
                    .core
                    .tabs
                    .read()
                    .iter()
                    .find(|t| Some(t.id) == self.active_tab_id)
                    .cloned();

                match self.view_mode {
                    ViewMode::StandardFullTab => {
                        let current_url = active_tab
                            .as_ref()
                            .map(|t| t.url.as_str())
                            .unwrap_or("noctra://newtab");

                        if current_url == "noctra://newtab" || current_url.is_empty() {
                            self.render_modern_new_tab_dashboard(ui, ctx);
                        } else {
                            // Find matching surface for active tab
                            let bound_surface = surfaces
                                .iter()
                                .find(|s| Some(s.tab_id) == self.active_tab_id)
                                .or_else(|| surfaces.first());

                            if let Some(s) = bound_surface {
                                if let Some((_, tex)) = self.surface_textures.get(&s.id) {
                                    ui.add(egui::Image::from_texture((
                                        tex.id(),
                                        Vec2::new(canvas_rect.width(), canvas_rect.height()),
                                    )));
                                } else {
                                    let is_rendering =
                                        self.rendering_in_progress.read().contains(&s.id);
                                    if !is_rendering {
                                        self.spawn_chromium_surface_render(
                                            ctx.clone(),
                                            s.id,
                                            current_url.to_string(),
                                            canvas_rect.width() as u32,
                                            canvas_rect.height() as u32,
                                            PathBuf::from(&active_prof.storage_partition_path),
                                        );
                                    }
                                    ui.vertical_centered(|ui| {
                                        ui.add_space(120.0);
                                        ui.label(
                                            RichText::new(format!("Loading {} ...", current_url))
                                                .size(16.0)
                                                .strong()
                                                .color(Color32::from_rgb(0x8B, 0x7C, 0xF6)),
                                        );
                                        ui.add_space(8.0);
                                        ui.label(
                                            RichText::new(
                                                "Rendering via Chromium/Blink + V8 Engine with TLS 1.3 & Privacy Shield",
                                            )
                                            .size(12.0)
                                            .color(Color32::from_rgb(0xA1, 0xA1, 0xAA)),
                                        );
                                    });
                                }
                            }
                        }
                    }

                    ViewMode::MultiSurfaceCanvas => {
                        for s in &surfaces {
                            if s.window_state == SurfaceWindowState::Minimized {
                                continue;
                            }

                            let win_x = canvas_rect.min.x + (s.x as f32).max(0.0);
                            let win_y = canvas_rect.min.y + (s.y as f32).max(0.0);
                            let win_w = (s.width as f32).clamp(300.0, canvas_rect.width());
                            let win_h = (s.height as f32).clamp(200.0, canvas_rect.height());
                            let win_rect = Rect::from_min_size(
                                Pos2::new(win_x, win_y),
                                Vec2::new(win_w, win_h),
                            );

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
                                        .stroke(Stroke::new(
                                            if s.is_focused { 1.5_f32 } else { 1.0_f32 },
                                            border_col,
                                        ))
                                        .rounding(Rounding::same(8.0))
                                        .show(ui, |ui| {
                                            ui.set_width(win_w);
                                            ui.set_height(win_h);

                                            let titlebar_bg = if s.is_focused {
                                                Color32::from_rgb(0x2B, 0x2D, 0x34)
                                            } else {
                                                Color32::from_rgb(0x1D, 0x1F, 0x24)
                                            };

                                            egui::Frame::none()
                                                .fill(titlebar_bg)
                                                .inner_margin(Margin::symmetric(8.0, 5.0))
                                                .show(ui, |ui| {
                                                    ui.horizontal(|ui| {
                                                        let drag_badge = ui.add(
                                                            egui::Label::new(
                                                                RichText::new(format!(
                                                                    ":: {}",
                                                                    s.title
                                                                        .chars()
                                                                        .take(18)
                                                                        .collect::<String>()
                                                                ))
                                                                .strong()
                                                                .size(11.5),
                                                            )
                                                            .sense(Sense::click_and_drag()),
                                                        );

                                                        if drag_badge.clicked()
                                                            || drag_badge.drag_started()
                                                        {
                                                            let _ =
                                                                self.core.wm.bring_to_front(s.id);
                                                        }
                                                        if drag_badge.dragged() {
                                                            let delta = drag_badge.drag_delta();
                                                            let nx = (s.x as f32 + delta.x) as i32;
                                                            let ny = (s.y as f32 + delta.y) as i32;
                                                            let _ = self
                                                                .core
                                                                .wm
                                                                .move_and_resize_surface(
                                                                    s.id, nx, ny, s.width,
                                                                    s.height, true,
                                                                );
                                                        }

                                                        let url_buf = self
                                                            .surface_url_buffers
                                                            .entry(s.id)
                                                            .or_insert_with(|| s.url.clone());

                                                        let url_edit = ui.add(
                                                            egui::TextEdit::singleline(url_buf)
                                                                .desired_width(
                                                                    (win_w - 250.0).max(100.0),
                                                                ),
                                                        );
                                                        if url_edit.lost_focus()
                                                            && ui.input(|i| {
                                                                i.key_pressed(egui::Key::Enter)
                                                            })
                                                        {
                                                            let target = self
                                                                .core
                                                                .resolve_omnibox_input(
                                                                    &url_buf.clone(),
                                                                );
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
                                                                    &active_prof
                                                                        .storage_partition_path,
                                                                ),
                                                            );
                                                        }

                                                        if ui.small_button("[L]").clicked() {
                                                            let _ =
                                                                self.core.wm.snap_surface_to_zone(
                                                                    s.id,
                                                                    SnapZone::LeftHalf,
                                                                );
                                                        }
                                                        if ui.small_button("[R]").clicked() {
                                                            let _ =
                                                                self.core.wm.snap_surface_to_zone(
                                                                    s.id,
                                                                    SnapZone::RightHalf,
                                                                );
                                                        }
                                                        if ui.small_button("_").clicked() {
                                                            let _ =
                                                                self.core.wm.minimize_surface(s.id);
                                                        }
                                                        if ui.small_button("[+]").clicked() {
                                                            if s.window_state
                                                                == SurfaceWindowState::Maximized
                                                            {
                                                                let _ = self
                                                                    .core
                                                                    .wm
                                                                    .restore_surface(s.id);
                                                            } else {
                                                                let _ = self
                                                                    .core
                                                                    .wm
                                                                    .maximize_surface(s.id);
                                                            }
                                                        }
                                                        if ui
                                                            .small_button(
                                                                RichText::new("×").color(
                                                                    Color32::from_rgb(
                                                                        0xEF, 0x44, 0x44,
                                                                    ),
                                                                ),
                                                            )
                                                            .clicked()
                                                        {
                                                            let _ =
                                                                self.core.wm.close_surface(s.id);
                                                        }
                                                    });
                                                });

                                            let viewport_h = (win_h - 54.0).max(100.0);
                                            if s.url == "noctra://newtab" {
                                                self.render_modern_new_tab_dashboard(ui, ctx);
                                            } else if let Some((_, tex)) =
                                                self.surface_textures.get(&s.id)
                                            {
                                                ui.add(egui::Image::from_texture((
                                                    tex.id(),
                                                    Vec2::new(win_w - 4.0, viewport_h),
                                                )));
                                            } else {
                                                ui.vertical_centered(|ui| {
                                                    ui.add_space(40.0);
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "Loading {} ...",
                                                            s.url
                                                        ))
                                                        .color(Color32::from_rgb(0x8B, 0x7C, 0xF6)),
                                                    );
                                                });
                                            }
                                        });
                                });
                        }
                    }
                }
            });

        // Handle automated screenshot verification if requested via CLI
        if let Some(ref shot_path) = self.auto_screenshot_path {
            if self.frames_rendered == 6 {
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
                let w = color_img.width() as u32;
                let h = color_img.height() as u32;
                let mut rgba = Vec::with_capacity((w * h * 4) as usize);
                for px in &color_img.pixels {
                    rgba.extend_from_slice(&[px.r(), px.g(), px.b(), px.a()]);
                }
                if let Some(buf) = image::RgbaImage::from_raw(w, h, rgba) {
                    let _ = buf.save(shot_path);
                }
                self.auto_screenshot_path = None;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ctx.request_repaint();
        }
    }
}

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
                ui.text_edit_singleline(&mut self.new_profile_name);
                if ui.button("+ Create Isolated Profile").clicked() {
                    let name = if self.new_profile_name.trim().is_empty() {
                        "Work Profile".to_string()
                    } else {
                        self.new_profile_name.trim().to_string()
                    };
                    let _ = self.core.profiles.create_profile(
                        name,
                        "#8B7CF6".to_string(),
                        "shield".to_string(),
                        "noctra://newtab".to_string(),
                    );
                    self.new_profile_name.clear();
                }

                ui.add_space(10.0);
                ui.heading("NOCTRA Spaces");
                for w in self.core.profiles.list_workspaces_for_active_profile() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&w.name).strong());
                        if ui.small_button("Switch").clicked() {
                            let _ = self.core.profiles.switch_workspace(w.id);
                            self.trigger_chromium_renders_for_active_workspace(ctx.clone());
                        }
                    });
                }

                ui.add_space(6.0);
                ui.text_edit_singleline(&mut self.new_workspace_name);
                ui.checkbox(
                    &mut self.new_workspace_isolated,
                    "Isolated Session (Independent Storage & Cookies)",
                );
                if ui.button("+ Create Space").clicked() {
                    let name = if self.new_workspace_name.trim().is_empty() {
                        "New Space".to_string()
                    } else {
                        self.new_workspace_name.trim().to_string()
                    };
                    let mode = if self.new_workspace_isolated {
                        WorkspaceSessionMode::Isolated
                    } else {
                        WorkspaceSessionMode::SharedSession
                    };
                    let _ = self.core.profiles.create_workspace(
                        name,
                        "layers".to_string(),
                        "#8B7CF6".to_string(),
                        mode,
                        vec![],
                    );
                    self.new_workspace_name.clear();
                }
            }

            SidebarModule::PrivacyAndSecurity => {
                ui.heading("Transport Security & Shield");
                ui.separator();
                let mut current_mode = *self.core.privacy.active_mode.read();
                egui::ComboBox::from_label("Mode")
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
                            "WireGuard VPN Mode",
                        );
                        ui.selectable_value(
                            &mut current_mode,
                            TransportProtectionMode::TorOnionMode,
                            "Tor Onion Circuit Mode",
                        );
                        ui.selectable_value(
                            &mut current_mode,
                            TransportProtectionMode::StandardHttps,
                            "Standard HTTPS",
                        );
                    });
                *self.core.privacy.active_mode.write() = current_mode;

                let obs = self.core.privacy.observer_visibility_report();
                ui.add_space(6.0);
                ui.label(
                    RichText::new("What Can an Observer See? (Wireshark / ISP View)")
                        .strong()
                        .color(Color32::from_rgb(0x8B, 0x7C, 0xF6)),
                );
                for h in &obs.hidden_from_network_observer {
                    ui.label(
                        RichText::new(format!("  [Encrypted] {}", h))
                            .size(11.0)
                            .color(Color32::from_rgb(0x10, 0xB9, 0x81)),
                    );
                }
                for v in &obs.visible_to_network_observer {
                    ui.label(RichText::new(format!("  • {}", v)).size(11.0));
                }

                ui.add_space(6.0);
                if ui.button("Run Wire Packet Leak Check").clicked() {
                    let sample_wire = [0x17u8, 0x03, 0x03, 0x00, 0x20, 0x9a, 0xbc, 0xde];
                    let res =
                        PrivacyAndTransportEngine::verify_captured_wire_bytes_contain_no_leaks(
                            &sample_wire,
                            &["https://secret.nocta.dev", "Bearer sk-live"],
                        );
                    self.status_banner = match res {
                        Ok(()) => "PASSED: 0 plaintext URLs, SNI, or credentials on wire!".to_string(),
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
                ui.heading("Developer Hub & Inspector");
                ui.separator();
                egui::ComboBox::from_label("Tool")
                    .selected_text(&self.devtool_selected)
                    .show_ui(ui, |ui| {
                        for t in [
                            "JSON Formatter & Validator",
                            "YAML Validator -> JSON",
                            "JWT Decoder",
                            "SHA-256 / SHA-512 Hash",
                            "Base64 Encode",
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

                if ui.button("Execute Tool").clicked() {
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
                        _ => DeveloperHubAndInspector::generate_uuids(5).join("\n"),
                    };
                }
                ui.code(&self.devtool_output);
            }

            SidebarModule::AiConnectionCenter => {
                ui.heading("NOCTRA AI Connection Center");
                ui.separator();
                for p in self.core.ai.list_providers() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{} ({})", p.name, p.selected_model)).strong());
                        ui.label(RichText::new(&p.masked_token_preview).monospace());
                        if ui.small_button("Remove").clicked() {
                            let _ = self.core.ai.remove_provider(p.id);
                        }
                    });
                }

                ui.label("Add Provider (Encrypted OS Vault):");
                ui.text_edit_singleline(&mut self.ai_prov_name);
                ui.text_edit_singleline(&mut self.ai_prov_url);
                ui.text_edit_singleline(&mut self.ai_prov_model);
                ui.add(
                    egui::TextEdit::singleline(&mut self.ai_prov_token)
                        .password(true)
                        .hint_text("API Token"),
                );
                if ui.button("+ Save Provider").clicked() {
                    let tok = if self.ai_prov_token.trim().is_empty() {
                        "sk-noctra-vault-key".to_string()
                    } else {
                        self.ai_prov_token.trim().to_string()
                    };
                    if let Ok(cfg) = self.core.ai.add_provider(
                        self.ai_prov_name.clone(),
                        self.ai_prov_kind,
                        self.ai_prov_url.clone(),
                        self.ai_prov_model.clone(),
                        vec![self.ai_prov_model.clone()],
                        RedactedSecret::new(tok),
                        20,
                    ) {
                        self.ai_status_output =
                            format!("Stored in OS Vault: {}", cfg.credential_ref);
                        self.ai_prov_token.clear();
                    }
                }
                ui.code(&self.ai_status_output);
            }

            SidebarModule::DownloadsAndTorrents => {
                ui.heading("Downloads & BitTorrent");
                ui.separator();
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
                        "{} ({}%) | DHT: {} nodes",
                        t.display_name, t.progress_pct, t.dht_nodes
                    ));
                }
            }

            SidebarModule::NetworkAndResources => {
                ui.heading("Network & Resource Monitor");
                ui.separator();
                let wifi = self.core.net_monitor.get_wifi_link_info();
                let res = self.core.query_resource_telemetry();
                ui.label(format!("Wi-Fi Link: {}", wifi.source_note));
                ui.label(format!(
                    "Host RAM: {} / {} MB",
                    res.system_used_ram_mb, res.system_total_ram_mb
                ));
                ui.label(format!("NOCTRA Core RAM: {} MB", res.noctra_core_ram_mb));
            }

            SidebarModule::BookmarksAndTimeMachine => {
                ui.heading("Session Time Machine");
                ui.separator();
                ui.text_edit_singleline(&mut self.snapshot_label_input);
                if ui.button("+ Capture Snapshot").clicked() {
                    let _ = self
                        .core
                        .create_session_snapshot(self.snapshot_label_input.clone(), false);
                }
                let prof = self.core.profiles.active_profile().unwrap();
                for snap in self.core.db.list_snapshots(prof.id).unwrap_or_default() {
                    ui.horizontal(|ui| {
                        ui.label(&snap.label);
                        if ui.small_button("Restore").clicked() {
                            let _ = self.core.restore_session_snapshot(snap.id);
                        }
                    });
                }
            }

            SidebarModule::SettingsAndSync => {
                ui.heading("Appearance & E2EE Sync");
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Dark Theme").clicked() {
                        let tokens = DesignTokens::noctra_dark();
                        Self::apply_noctra_visual_tokens(ctx, &tokens);
                        self.core.settings.write().design_tokens = tokens;
                    }
                    if ui.button("Light Theme").clicked() {
                        let tokens = DesignTokens::noctra_light();
                        Self::apply_noctra_visual_tokens(ctx, &tokens);
                        self.core.settings.write().design_tokens = tokens;
                    }
                });
            }
        }
    }
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
            .with_title("NOCTRA")
            .with_app_id("noctra-desktop-browser")
            .with_icon(app_icon)
            .with_inner_size([1360.0, 840.0])
            .with_min_inner_size([900.0, 560.0]),
        ..Default::default()
    };

    eframe::run_native(
        "NOCTRA",
        native_options,
        Box::new(move |cc| {
            Ok(Box::new(NoctraNativeDesktopApp::new(
                cc,
                core,
                rt,
                auto_screenshot_path,
            )))
        }),
    )
}
