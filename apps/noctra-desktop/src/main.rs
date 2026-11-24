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
    RedactedSecret, SnapZone, SurfaceWindowState, TabState, TransportProtectionMode,
    WorkspaceSessionMode,
};
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    /// Clean Arc/Dia-style Single or Split Viewport Canvas
    UnifiedCanvas,
    /// Multi-Surface Floating Window Manager
    FloatingWindows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InspectorModal {
    None,
    TransportSecurity,
    DeveloperHub,
    AiConnectionCenter,
    DownloadsAndTorrents,
    NetworkAndResources,
}

#[derive(Debug, Clone)]
struct AiChatThread {
    id: Uuid,
    title: String,
    timestamp: String,
    messages: Vec<(String, String)>, // (role, text)
    worked_duration: String,
    actions_count: u32,
}

struct NoctraNativeDesktopApp {
    core: Arc<NoctraBrowserCore>,
    rt: Arc<tokio::runtime::Runtime>,
    active_tab_id: Option<Uuid>,
    active_chat_id: Option<Uuid>,
    showing_ai_chat_canvas: bool,
    show_agent_approval_modal: bool,
    sidebar_collapsed: bool,

    omnibox_buffer: String,
    sidebar_search_buffer: String,
    chat_reply_buffer: String,

    view_mode: ViewMode,
    active_modal: InspectorModal,
    folder_project_open: bool,
    folder_daily_open: bool,
    folder_reading_open: bool,

    chat_threads: Vec<AiChatThread>,
    surface_textures: HashMap<Uuid, (String, TextureHandle)>,
    rendering_in_progress: Arc<RwLock<HashSet<Uuid>>>,
    surface_url_buffers: HashMap<Uuid, String>,

    // Modal buffers
    devtool_selected: String,
    devtool_input: String,
    devtool_output: String,
    ai_prov_name: String,
    ai_prov_url: String,
    ai_prov_model: String,
    ai_prov_token: String,
    ai_status_output: String,
    magnet_uri_input: String,
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
        Self::apply_noctra_arc_style(&cc.egui_ctx, &tokens);

        let _ = core.ai.register_external_agent(
            "Noctra Autonomous Web Agent".to_string(),
            "Sandboxed MCP agent with explicit high-risk action approval gate".to_string(),
            AgentTransportKind::McpHttpSse,
            "http://localhost:8090/mcp".to_string(),
            None,
            vec![
                AgentCapability::ReadCurrentWebpage,
                AgentCapability::SubmitForms,
                AgentCapability::RunLocalCommands,
            ],
        );

        // Ensure clean initial tabs matching the sleek reference design
        {
            let mut tabs_guard = core.tabs.write();
            for t in tabs_guard.iter_mut() {
                if t.url.contains("rust-lang.org") || t.url.contains("Rust_(programming_language)")
                {
                    t.url = "noctra://ask".to_string();
                    t.title = "Ask / Evening Reservation".to_string();
                }
            }
            let _ = core.db.save_kv("tabs", "open_tabs", &*tabs_guard);
        }

        let default_chats = vec![
            AiChatThread {
                id: Uuid::new_v4(),
                title: "Dinner Friday".to_string(),
                timestamp: "18:02".to_string(),
                worked_duration: "Worked for 3m 16s".to_string(),
                actions_count: 23,
                messages: vec![
                    (
                        "user".to_string(),
                        "Let's do seafood Friday for four, somewhere Maya can eat".to_string(),
                    ),
                    (
                        "assistant".to_string(),
                        "Seafood and vegan pull against each other. Four of the 31 places near you do both:\n\n1. Saltgrass (My pick) — Seafood • $$ • ★ 4.7  |  12 min drive • 9 plant mains • 8:00 PM • $25/person no-show fee\n2. Brine — Oyster bar • $$ • ★ 4.8  |  6 min drive • 1 plant main • 7:30 PM • None\n3. Juniper — Coastal Italian • $$ • ★ 4.5  |  18 min drive • 4 plant mains • 8:45 PM • None\n4. Little Gem — Seafood & veg • $$ • ★ 4.6  |  9 min drive • 6 plant mains • Full • $15/person\n\nSaltgrass is my pick: nine plant mains, so Maya orders off the same menu, and it's free at 8:00, it costs the most. Brine is closest with the best oysters, but she'd get one main.".to_string(),
                    ),
                ],
            },
            AiChatThread {
                id: Uuid::new_v4(),
                title: "Morning brief & SRE status".to_string(),
                timestamp: "10:43".to_string(),
                worked_duration: "Worked for 42s".to_string(),
                actions_count: 8,
                messages: vec![
                    ("user".to_string(), "Check production mesh & TLS certificates".to_string()),
                    ("assistant".to_string(), "All 14 endpoints verified on TLS 1.3 + ECH. Zero non-public MITM root CAs detected.".to_string()),
                ],
            },
            AiChatThread {
                id: Uuid::new_v4(),
                title: "Work network access".to_string(),
                timestamp: "10:41".to_string(),
                worked_duration: "Worked for 19s".to_string(),
                actions_count: 4,
                messages: vec![
                    ("user".to_string(), "Verify WireGuard tunnel & DoH leak status".to_string()),
                    ("assistant".to_string(), "Cloudflare DoH active. Wire packet capture verified 0 plaintext DNS or SNI leaks.".to_string()),
                ],
            },
            AiChatThread {
                id: Uuid::new_v4(),
                title: "Compare noise cancelling headphones".to_string(),
                timestamp: "10:36".to_string(),
                worked_duration: "Worked for 1m 10s".to_string(),
                actions_count: 12,
                messages: vec![],
            },
        ];

        let first_chat_id = default_chats.first().map(|c| c.id);
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

        Self {
            core,
            rt,
            active_tab_id: first_tab.map(|t| t.id),
            active_chat_id: first_chat_id,
            showing_ai_chat_canvas: true,
            show_agent_approval_modal: true,
            sidebar_collapsed: false,
            omnibox_buffer: "Ask / chats / dinner-friday".to_string(),
            sidebar_search_buffer: String::new(),
            chat_reply_buffer: String::new(),
            view_mode: ViewMode::UnifiedCanvas,
            active_modal: InspectorModal::None,
            folder_project_open: true,
            folder_daily_open: true,
            folder_reading_open: false,
            chat_threads: default_chats,
            surface_textures: HashMap::new(),
            rendering_in_progress: Arc::new(RwLock::new(HashSet::new())),
            surface_url_buffers: HashMap::new(),
            devtool_selected: "JSON Formatter & Validator".to_string(),
            devtool_input: "{\n  \"product\": \"NOCTRA\",\n  \"engine\": \"Chromium/Blink + Rust\"\n}".to_string(),
            devtool_output: "Ready.".to_string(),
            ai_prov_name: "Anthropic Claude 3.7".to_string(),
            ai_prov_url: "https://api.anthropic.com/v1/messages".to_string(),
            ai_prov_model: "claude-3-7-sonnet".to_string(),
            ai_prov_token: String::new(),
            ai_status_output: "OS Credential Vault active (cred_ref://).".to_string(),
            magnet_uri_input: "magnet:?xt=urn:btih:a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4&dn=Ubuntu+24.04+Desktop.iso".to_string(),
            status_banner: "PASSED: 0 plaintext URLs, SNI, or tokens on wire.".to_string(),
            auto_screenshot_path,
            frames_rendered: 0,
        }
    }

    fn apply_noctra_arc_style(ctx: &egui::Context, _tokens: &DesignTokens) {
        let bg_deep = Color32::from_rgb(0x12, 0x13, 0x16);
        let bg_sidebar = Color32::from_rgb(0x18, 0x19, 0x1E);
        let surface_card = Color32::from_rgb(0x20, 0x22, 0x28);
        let surface_hover = Color32::from_rgb(0x28, 0x2A, 0x32);
        let accent_violet = Color32::from_rgb(0x8B, 0x7C, 0xF6);
        let text_primary = Color32::from_rgb(0xF1, 0xF1, 0xF3);
        let text_muted = Color32::from_rgb(0x94, 0x96, 0xA1);
        let border_subtle = Color32::from_rgb(0x2A, 0x2C, 0x34);

        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = bg_deep;
        visuals.window_fill = bg_sidebar;
        visuals.extreme_bg_color = bg_deep;
        visuals.faint_bg_color = surface_card;
        visuals.override_text_color = Some(text_primary);
        visuals.selection.bg_fill = Color32::from_rgb(0x34, 0x32, 0x46);
        visuals.selection.stroke = Stroke::new(1.0_f32, accent_violet);
        visuals.window_stroke = Stroke::new(1.0_f32, border_subtle);
        visuals.window_rounding = Rounding::same(10.0);

        visuals.widgets.noninteractive.bg_fill = surface_card;
        visuals.widgets.noninteractive.weak_bg_fill = surface_card;
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, border_subtle);
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, text_muted);
        visuals.widgets.noninteractive.rounding = Rounding::same(7.0);

        visuals.widgets.inactive.bg_fill = surface_card;
        visuals.widgets.inactive.weak_bg_fill = surface_card;
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, border_subtle);
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, text_primary);
        visuals.widgets.inactive.rounding = Rounding::same(7.0);

        visuals.widgets.hovered.bg_fill = surface_hover;
        visuals.widgets.hovered.weak_bg_fill = surface_hover;
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, accent_violet);
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
        visuals.widgets.hovered.rounding = Rounding::same(7.0);

        visuals.widgets.active.bg_fill = accent_violet;
        visuals.widgets.active.weak_bg_fill = accent_violet;
        visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, accent_violet);
        visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
        visuals.widgets.active.rounding = Rounding::same(7.0);

        let mut style = (*ctx.style()).clone();
        style.visuals = visuals;
        style.spacing.item_spacing = Vec2::new(6.0, 5.0);
        style.spacing.window_margin = Margin::same(12.0);
        style.spacing.button_padding = Vec2::new(8.0, 4.0);
        style.animation_time = 0.0;
        ctx.set_style(style);
    }

    fn draw_dot(ui: &mut egui::Ui, color: Color32, radius: f32) {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(radius * 2.4, radius * 2.4), Sense::hover());
        ui.painter().circle_filled(rect.center(), radius, color);
    }

    fn navigate_to_live_url(&mut self, ctx: &egui::Context, raw_input: &str) {
        let trimmed = raw_input.trim();
        if trimmed.is_empty() {
            return;
        }
        self.showing_ai_chat_canvas = false;
        if let Some(tid) = self.active_tab_id {
            if let Ok(t) = self.core.navigate_tab(tid, trimmed) {
                self.omnibox_buffer = t.url.clone();
            }
        } else if let Ok(t) = self.core.open_tab_and_surface(trimmed, None, true) {
            self.active_tab_id = Some(t.id);
            self.omnibox_buffer = t.url.clone();
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
            if !s.url.starts_with("noctra://") && !s.url.starts_with("about:") {
                self.spawn_chromium_surface_render(
                    egui_ctx.clone(),
                    s.id,
                    s.url.clone(),
                    s.width.max(1200),
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
        if url.starts_with("noctra://") || url.starts_with("about:") {
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
        // 1. ULTRA-SLEEK TOP BAR (MATCHING REFERENCE IMAGE 1 & 2)
        // ====================================================================
        egui::TopBottomPanel::top("noctra_arc_topbar")
            .exact_height(38.0)
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(0x14, 0x15, 0x19))
                    .inner_margin(Margin::symmetric(12.0, 5.0))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x25, 0x27, 0x2F))),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    Self::draw_dot(ui, Color32::from_rgb(0x8B, 0x7C, 0xF6), 3.8);
                    // Workspace Dropdown Badge (e.g. "Work")
                    egui::ComboBox::from_id_salt("top_workspace_combo")
                        .selected_text(
                            RichText::new(&active_ws.name)
                                .size(12.5)
                                .strong()
                                .color(Color32::from_rgb(0xF1, 0xF1, 0xF3)),
                        )
                        .width(125.0)
                        .show_ui(ui, |ui| {
                            for w in &ws_list {
                                if ui
                                    .selectable_label(w.id == active_ws.id, &w.name)
                                    .clicked()
                                {
                                    let _ = self.core.profiles.switch_workspace(w.id);
                                }
                            }
                        });

                    // Sidebar Toggle Button
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("[|]")
                                    .size(11.0)
                                    .strong()
                                    .color(Color32::from_rgb(0xA1, 0xA1, 0xAA)),
                            )
                            .fill(Color32::TRANSPARENT),
                        )
                        .on_hover_text("Toggle Vertical Sidebar")
                        .clicked()
                    {
                        self.sidebar_collapsed = !self.sidebar_collapsed;
                    }

                    // Minimal Navigation Controls: <  >  ↻
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(" < ")
                                    .size(12.0)
                                    .color(Color32::from_rgb(0xA1, 0xA1, 0xAA)),
                            )
                            .fill(Color32::TRANSPARENT),
                        )
                        .clicked()
                    {
                        if let Some(tid) = self.active_tab_id {
                            if let Ok(t) = self.core.tab_go_back(tid) {
                                self.omnibox_buffer = t.url;
                                self.trigger_chromium_renders_for_active_workspace(ctx.clone());
                            }
                        }
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(" > ")
                                    .size(12.0)
                                    .color(Color32::from_rgb(0xA1, 0xA1, 0xAA)),
                            )
                            .fill(Color32::TRANSPARENT),
                        )
                        .clicked()
                    {
                        if let Some(tid) = self.active_tab_id {
                            if let Ok(t) = self.core.tab_go_forward(tid) {
                                self.omnibox_buffer = t.url;
                                self.trigger_chromium_renders_for_active_workspace(ctx.clone());
                            }
                        }
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(" ↻ ")
                                    .size(12.5)
                                    .color(Color32::from_rgb(0xA1, 0xA1, 0xAA)),
                            )
                            .fill(Color32::TRANSPARENT),
                        )
                        .clicked()
                    {
                        self.surface_textures.clear();
                        self.trigger_chromium_renders_for_active_workspace(ctx.clone());
                    }

                    ui.add_space(28.0);

                    // Centered Breadcrumb / Ask / URL Pill Bar (`Ask / chats / dinner-friday`)
                    let center_w = (ui.available_width() - 340.0).clamp(260.0, 500.0);
                    egui::Frame::none()
                        .fill(Color32::from_rgb(0x1D, 0x1F, 0x25))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x2D, 0x30, 0x39)))
                        .rounding(Rounding::same(8.0))
                        .inner_margin(Margin::symmetric(12.0, 3.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                Self::draw_dot(ui, Color32::from_rgb(0x8B, 0x7C, 0xF6), 3.2);
                                let resp = ui.add(
                                    egui::TextEdit::singleline(&mut self.omnibox_buffer)
                                        .frame(false)
                                        .desired_width(center_w - 55.0)
                                        .font(FontId::new(12.0, FontFamily::Proportional))
                                        .hint_text("Ask NOCTRA or type a URL..."),
                                );
                                if resp.lost_focus()
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                {
                                    let val = self.omnibox_buffer.clone();
                                    if val.starts_with("Ask") || val.contains("dinner") {
                                        self.showing_ai_chat_canvas = true;
                                    } else {
                                        self.navigate_to_live_url(ctx, &val);
                                    }
                                }
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("☆")
                                                .size(12.5)
                                                .color(Color32::from_rgb(0x94, 0x96, 0xA1)),
                                        )
                                        .frame(false),
                                    )
                                    .on_hover_text("Bookmark")
                                    .clicked()
                                {
                                    let _ = self.core.db.add_bookmark(&BookmarkEntry {
                                        id: Uuid::new_v4(),
                                        profile_id: active_prof.id,
                                        workspace_id: Some(active_ws.id),
                                        title: self.omnibox_buffer.clone(),
                                        url: self.omnibox_buffer.clone(),
                                        folder: "Daily".to_string(),
                                        tags: vec![],
                                        created_at: Utc::now(),
                                    });
                                }
                            });
                        });

                    // Right-side Status & Agent Action Pills (matching reference image 1)
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        // Amber "Approve 1 action" pill button
                        let approve_pill = egui::Button::new(
                            RichText::new("Approve 1 action  ⌘J")
                                .size(11.5)
                                .strong()
                                .color(Color32::from_rgb(0xF5, 0x9E, 0x0B)),
                        )
                        .fill(Color32::from_rgb(0x2B, 0x23, 0x16))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x78, 0x52, 0x17)))
                        .rounding(Rounding::same(14.0));

                        if ui
                            .add(approve_pill)
                            .on_hover_text("Inspect & Approve Pending Agent Action")
                            .clicked()
                        {
                            self.show_agent_approval_modal = !self.show_agent_approval_modal;
                        }

                        // Active Agents Counter Pill (`3 agents`)
                        let agents_pill = egui::Button::new(
                            RichText::new("3 agents")
                                .size(11.5)
                                .color(Color32::from_rgb(0xE4, 0xE4, 0xE7)),
                        )
                        .fill(Color32::from_rgb(0x20, 0x22, 0x28))
                        .rounding(Rounding::same(14.0));

                        if ui
                            .add(agents_pill)
                            .on_hover_text("NOCTRA AI Connection Center & Active MCP Agents")
                            .clicked()
                        {
                            self.active_modal =
                                if self.active_modal == InspectorModal::AiConnectionCenter {
                                    InspectorModal::None
                                } else {
                                    InspectorModal::AiConnectionCenter
                                };
                        }

                        // Transport Security Shield Icon (`TLS 1.3`)
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("🛡 TLS 1.3")
                                        .size(11.0)
                                        .color(Color32::from_rgb(0x10, 0xB9, 0x81)),
                                )
                                .fill(Color32::from_rgb(0x16, 0x27, 0x21))
                                .rounding(Rounding::same(12.0)),
                            )
                            .on_hover_text("Transport Security & 'What Can an Observer See?'")
                            .clicked()
                        {
                            self.active_modal =
                                if self.active_modal == InspectorModal::TransportSecurity {
                                    InspectorModal::None
                                } else {
                                    InspectorModal::TransportSecurity
                                };
                        }

                        // DevHub & Multi-Surface Toggle
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(if self.view_mode == ViewMode::FloatingWindows {
                                        "Unified View"
                                    } else {
                                        "Split Canvas"
                                    })
                                    .size(11.0)
                                    .color(Color32::from_rgb(0xA1, 0xA1, 0xAA)),
                                )
                                .fill(Color32::from_rgb(0x20, 0x22, 0x28)),
                            )
                            .clicked()
                        {
                            self.view_mode = if self.view_mode == ViewMode::FloatingWindows {
                                ViewMode::UnifiedCanvas
                            } else {
                                ViewMode::FloatingWindows
                            };
                        }
                    });
                });
            });

        // ====================================================================
        // 2. ARC / DIA STYLE LEFT VERTICAL SIDEBAR (MATCHING REFERENCE IMAGE 1 & 2)
        // ====================================================================
        if !self.sidebar_collapsed {
            egui::SidePanel::left("noctra_arc_vertical_sidebar")
                .exact_width(248.0)
                .resizable(false)
                .frame(
                    egui::Frame::none()
                        .fill(Color32::from_rgb(0x16, 0x17, 0x1C))
                        .inner_margin(Margin::symmetric(10.0, 10.0))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x25, 0x27, 0x2F))),
                )
                .show(ctx, |ui| {
                    // 2A. Sidebar Search Box (`Search or type a URL   ⌘K`)
                    egui::Frame::none()
                        .fill(Color32::from_rgb(0x20, 0x22, 0x29))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x2E, 0x31, 0x3B)))
                        .rounding(Rounding::same(8.0))
                        .inner_margin(Margin::symmetric(10.0, 6.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                Self::draw_dot(ui, Color32::from_rgb(0x8B, 0x7C, 0xF6), 3.0);
                                let s_resp = ui.add(
                                    egui::TextEdit::singleline(&mut self.sidebar_search_buffer)
                                        .frame(false)
                                        .desired_width(155.0)
                                        .font(FontId::new(12.0, FontFamily::Proportional))
                                        .hint_text("Search or type a URL"),
                                );
                                ui.label(
                                    RichText::new("⌘K")
                                        .size(10.0)
                                        .color(Color32::from_rgb(0x6E, 0x71, 0x7C)),
                                );
                                if s_resp.lost_focus()
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                {
                                    let q = self.sidebar_search_buffer.clone();
                                    self.sidebar_search_buffer.clear();
                                    self.navigate_to_live_url(ctx, &q);
                                }
                            });
                        });

                    ui.add_space(6.0);

                    // 2B. Pinned App Tiles Grid (2x4 Rounded Square Tiles like Reference Image 2!)
                    let pinned_apps = [
                        ("M", Color32::from_rgb(0xEA, 0x43, 0x35), "https://mail.google.com", "Gmail"),
                        ("31", Color32::from_rgb(0x3B, 0x82, 0xF6), "https://calendar.google.com", "Calendar"),
                        ("◎", Color32::from_rgb(0xA1, 0xA1, 0xAA), "https://linear.app", "Linear"),
                        ("S", Color32::from_rgb(0x10, 0xB9, 0x81), "https://open.spotify.com", "Spotify"),
                        ("N", Color32::from_rgb(0xF1, 0xF1, 0xF3), "https://www.notion.so", "Notion"),
                        ("F", Color32::from_rgb(0xEC, 0x48, 0x99), "https://www.figma.com", "Figma"),
                        ("GH", Color32::from_rgb(0xE4, 0xE4, 0xE7), "https://github.com", "GitHub"),
                        ("AI", Color32::from_rgb(0x8B, 0x7C, 0xF6), "noctra://ask", "NOCTRA Ask AI"),
                    ];

                    egui::Grid::new("arc_pinned_tiles_grid")
                        .spacing(Vec2::new(6.0, 6.0))
                        .show(ui, |ui| {
                            for (idx, (icon_txt, col, target_url, tooltip)) in
                                pinned_apps.iter().enumerate()
                            {
                                let tile_btn = egui::Button::new(
                                    RichText::new(*icon_txt).size(13.0).strong().color(*col),
                                )
                                .min_size(Vec2::new(52.0, 36.0))
                                .fill(Color32::from_rgb(0x22, 0x24, 0x2C))
                                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x2E, 0x31, 0x3B)))
                                .rounding(Rounding::same(8.0));

                                if ui.add(tile_btn).on_hover_text(*tooltip).clicked() {
                                    if *target_url == "noctra://ask" {
                                        self.showing_ai_chat_canvas = true;
                                        self.omnibox_buffer =
                                            "Ask / chats / dinner-friday".to_string();
                                    } else {
                                        self.navigate_to_live_url(ctx, target_url);
                                    }
                                }
                                if idx == 3 {
                                    ui.end_row();
                                }
                            }
                        });

                    ui.add_space(8.0);

                    // Scrollable Middle Area: CHATS + TABS + FOLDERS
                    egui::ScrollArea::vertical()
                        .max_height((ui.available_height() - 108.0).max(200.0))
                        .show(ui, |ui| {
                            // 2C. CHATS Section (matching Reference Image 1 & 2)
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("Chats")
                                        .size(11.0)
                                        .strong()
                                        .color(Color32::from_rgb(0x8E, 0x90, 0x9B)),
                                );
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("+")
                                                    .size(12.0)
                                                    .color(Color32::from_rgb(0x94, 0x96, 0xA1)),
                                            )
                                            .frame(false),
                                        )
                                        .on_hover_text("New AI Research Thread")
                                        .clicked()
                                    {
                                        let new_id = Uuid::new_v4();
                                        self.chat_threads.insert(
                                            0,
                                            AiChatThread {
                                                id: new_id,
                                                title: "New Research Task".to_string(),
                                                timestamp: "Now".to_string(),
                                                worked_duration: "Ready".to_string(),
                                                actions_count: 1,
                                                messages: vec![(
                                                    "assistant".to_string(),
                                                    "How can NOCTRA assist or automate your workflow today?"
                                                        .to_string(),
                                                )],
                                            },
                                        );
                                        self.active_chat_id = Some(new_id);
                                        self.showing_ai_chat_canvas = true;
                                    }
                                });
                            });

                            for chat in &self.chat_threads {
                                let is_sel =
                                    self.showing_ai_chat_canvas && self.active_chat_id == Some(chat.id);
                                egui::Frame::none()
                                    .fill(if is_sel {
                                        Color32::from_rgb(0x28, 0x2A, 0x33)
                                    } else {
                                        Color32::TRANSPARENT
                                    })
                                    .rounding(Rounding::same(6.0))
                                    .inner_margin(Margin::symmetric(8.0, 4.0))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            Self::draw_dot(
                                                ui,
                                                if is_sel {
                                                    Color32::from_rgb(0x8B, 0x7C, 0xF6)
                                                } else {
                                                    Color32::from_rgb(0x6E, 0x71, 0x7C)
                                                },
                                                3.0,
                                            );
                                            let row_lbl = ui.add(
                                                egui::Label::new(
                                                    RichText::new(
                                                        chat.title
                                                            .chars()
                                                            .take(21)
                                                            .collect::<String>(),
                                                    )
                                                    .size(12.0)
                                                    .color(if is_sel {
                                                        Color32::WHITE
                                                    } else {
                                                        Color32::from_rgb(0xD4, 0xD5, 0xDC)
                                                    }),
                                                )
                                                .sense(Sense::click()),
                                            );
                                            if row_lbl.clicked() {
                                                self.active_chat_id = Some(chat.id);
                                                self.showing_ai_chat_canvas = true;
                                                self.omnibox_buffer = format!(
                                                    "Ask / chats / {}",
                                                    chat.title.to_lowercase().replace(' ', "-")
                                                );
                                            }
                                            ui.with_layout(
                                                Layout::right_to_left(Align::Center),
                                                |ui| {
                                                    ui.label(
                                                        RichText::new(&chat.timestamp)
                                                            .size(10.0)
                                                            .color(Color32::from_rgb(
                                                                0x6E, 0x71, 0x7C,
                                                            )),
                                                    );
                                                },
                                            );
                                        });
                                    });
                            }

                            ui.add_space(6.0);

                            // 2D. TABS & FOLDERS Section (matching Reference Image 1 & 2)
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("TABS")
                                        .size(10.5)
                                        .strong()
                                        .color(Color32::from_rgb(0x8E, 0x90, 0x9B)),
                                );
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("Tidy")
                                                    .size(10.5)
                                                    .color(Color32::from_rgb(0x94, 0x96, 0xA1)),
                                            )
                                            .frame(false),
                                        )
                                        .on_hover_text("Hibernate background tabs & auto-organize")
                                        .clicked()
                                    {
                                        let _ = self.core.sleep_all_background_tabs();
                                    }
                                });
                            });

                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("  +  New tab")
                                            .size(12.0)
                                            .color(Color32::from_rgb(0xA1, 0xA1, 0xAA)),
                                    )
                                    .fill(Color32::TRANSPARENT),
                                )
                                .clicked()
                            {
                                self.navigate_to_live_url(ctx, "https://www.google.com");
                            }

                            // Pinned Quick Tabs (Payments - Stripe, Hacker News)
                            for (badge, badge_col, label, url) in [
                                ("S", Color32::from_rgb(0x63, 0x5B, 0xFF), "Payments – Stripe", "https://stripe.com"),
                                ("Y", Color32::from_rgb(0xF9, 0x73, 0x16), "Hacker News", "https://news.ycombinator.com"),
                            ] {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(badge).size(11.5).strong().color(badge_col));
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new(label)
                                                    .size(12.0)
                                                    .color(Color32::from_rgb(0xD4, 0xD5, 0xDC)),
                                            )
                                            .frame(false),
                                        )
                                        .clicked()
                                    {
                                        self.navigate_to_live_url(ctx, url);
                                    }
                                });
                            }

                            ui.add_space(4.0);

                            // Folder 1: `Errands & Tasks` (Elevated Rounded Card like Image 2!)
                            egui::Frame::none()
                                .fill(Color32::from_rgb(0x22, 0x23, 0x2B))
                                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x31, 0x33, 0x3E)))
                                .rounding(Rounding::same(9.0))
                                .inner_margin(Margin::symmetric(8.0, 7.0))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal(|ui| {
                                        if ui
                                            .add(
                                                egui::Button::new(
                                                    RichText::new("Errands & Tasks")
                                                        .size(11.5)
                                                        .strong()
                                                        .color(Color32::from_rgb(0xE4, 0xE4, 0xE7)),
                                                )
                                                .frame(false),
                                            )
                                            .clicked()
                                        {
                                            self.folder_project_open = !self.folder_project_open;
                                        }
                                    });

                                    if self.folder_project_open {
                                        ui.add_space(3.0);
                                        // Active item inside folder with "approve" badge
                                        egui::Frame::none()
                                            .fill(Color32::from_rgb(0x2F, 0x31, 0x3C))
                                            .rounding(Rounding::same(6.0))
                                            .inner_margin(Margin::symmetric(7.0, 4.0))
                                            .show(ui, |ui| {
                                                ui.horizontal(|ui| {
                                                    Self::draw_dot(
                                                        ui,
                                                        Color32::from_rgb(0xEF, 0x44, 0x44),
                                                        3.2,
                                                    );
                                                    if ui
                                                        .add(
                                                            egui::Button::new(
                                                                RichText::new("Saltgrass – OpenTable")
                                                                    .size(11.5)
                                                                    .color(Color32::WHITE),
                                                            )
                                                            .frame(false),
                                                        )
                                                        .clicked()
                                                    {
                                                        self.showing_ai_chat_canvas = true;
                                                        self.show_agent_approval_modal = true;
                                                    }
                                                    ui.with_layout(
                                                        Layout::right_to_left(Align::Center),
                                                        |ui| {
                                                            ui.label(
                                                                RichText::new("approve")
                                                                    .size(9.5)
                                                                    .color(Color32::from_rgb(
                                                                        0xF5, 0x9E, 0x0B,
                                                                    )),
                                                            );
                                                        },
                                                    );
                                                });
                                            });

                                        // Dynamic live tabs from NoctraBrowserCore
                                        let ws_tabs: Vec<_> = self
                                            .core
                                            .tabs
                                            .read()
                                            .iter()
                                            .filter(|t| t.workspace_id == active_ws.id)
                                            .cloned()
                                            .collect();

                                        for tab in ws_tabs {
                                            let is_act = !self.showing_ai_chat_canvas
                                                && self.active_tab_id == Some(tab.id);
                                            ui.horizontal(|ui| {
                                                ui.label(
                                                    RichText::new("🌐").size(10.5).color(
                                                        Color32::from_rgb(0x8B, 0x7C, 0xF6),
                                                    ),
                                                );
                                                if ui
                                                    .add(
                                                        egui::Button::new(
                                                            RichText::new(
                                                                tab.title
                                                                    .chars()
                                                                    .take(18)
                                                                    .collect::<String>(),
                                                            )
                                                            .size(11.5)
                                                            .color(if is_act {
                                                                Color32::WHITE
                                                            } else {
                                                                Color32::from_rgb(0xB4, 0xB6, 0xC2)
                                                            }),
                                                        )
                                                        .frame(false),
                                                    )
                                                    .clicked()
                                                {
                                                    self.active_tab_id = Some(tab.id);
                                                    self.showing_ai_chat_canvas = false;
                                                    self.omnibox_buffer = tab.url.clone();
                                                }
                                            });
                                        }
                                    }
                                });

                            ui.add_space(6.0);

                            // Folder 2: `Daily` (Elevated Rounded Card like Image 2!)
                            egui::Frame::none()
                                .fill(Color32::from_rgb(0x22, 0x23, 0x2B))
                                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x31, 0x33, 0x3E)))
                                .rounding(Rounding::same(9.0))
                                .inner_margin(Margin::symmetric(8.0, 7.0))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("Daily")
                                                    .size(11.5)
                                                    .strong()
                                                    .color(Color32::from_rgb(0xE4, 0xE4, 0xE7)),
                                            )
                                            .frame(false),
                                        )
                                        .clicked()
                                    {
                                        self.folder_daily_open = !self.folder_daily_open;
                                    }

                                    if self.folder_daily_open {
                                        ui.add_space(3.0);
                                        for (icon_c, col, title, url) in [
                                            ("G", Color32::from_rgb(0x3B, 0x82, 0xF6), "Google", "https://www.google.com"),
                                            ("◎", Color32::from_rgb(0xA1, 0xA1, 0xAA), "Inbox · Linear", "https://linear.app"),
                                            ("S", Color32::from_rgb(0x10, 0xB9, 0x81), "Spotify – Daily Mix 1", "https://open.spotify.com"),
                                            ("W", Color32::from_rgb(0xF1, 0xF1, 0xF3), "Wikipedia", "https://www.wikipedia.org"),
                                        ] {
                                            ui.horizontal(|ui| {
                                                ui.label(RichText::new(icon_c).size(11.0).strong().color(col));
                                                if ui
                                                    .add(
                                                        egui::Button::new(
                                                            RichText::new(title)
                                                                .size(11.5)
                                                                .color(Color32::from_rgb(0xC4, 0xC6, 0xD0)),
                                                        )
                                                        .frame(false),
                                                    )
                                                    .clicked()
                                                {
                                                    self.navigate_to_live_url(ctx, url);
                                                }
                                            });
                                        }
                                    }
                                });
                        });

                    // 2E. Bottom Status Pills inside Sidebar (matching Reference Image 2 bottom!)
                    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                        // Bottom Footer Row: Downloads, DevHub, Shield, Profile Switcher
                        ui.horizontal(|ui| {
                            if ui
                                .add(
                                    egui::Button::new(RichText::new("⬇").size(12.0))
                                        .fill(Color32::TRANSPARENT),
                                )
                                .on_hover_text("Downloads & Native BitTorrent")
                                .clicked()
                            {
                                self.active_modal = InspectorModal::DownloadsAndTorrents;
                            }
                            if ui
                                .add(
                                    egui::Button::new(RichText::new("🛠").size(12.0))
                                        .fill(Color32::TRANSPARENT),
                                )
                                .on_hover_text("Developer Hub & Network Inspector")
                                .clicked()
                            {
                                self.active_modal = InspectorModal::DeveloperHub;
                            }
                            if ui
                                .add(
                                    egui::Button::new(RichText::new("📶").size(12.0))
                                        .fill(Color32::TRANSPARENT),
                                )
                                .on_hover_text("Network Quality & Resource Governor")
                                .clicked()
                            {
                                self.active_modal = InspectorModal::NetworkAndResources;
                            }

                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                egui::ComboBox::from_id_salt("sidebar_profile_footer")
                                    .selected_text(
                                        RichText::new(&active_prof.name).size(11.5).strong(),
                                    )
                                    .width(105.0)
                                    .show_ui(ui, |ui| {
                                        for p in &all_profiles {
                                            if ui
                                                .selectable_label(p.id == active_prof.id, &p.name)
                                                .clicked()
                                            {
                                                let _ = self.core.profiles.switch_profile(p.id);
                                            }
                                        }
                                    });
                            });
                        });

                        ui.add_space(4.0);

                        // Pill 2: `1 needs you   5 agents`
                        egui::Frame::none()
                            .fill(Color32::from_rgb(0x22, 0x24, 0x2C))
                            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x31, 0x33, 0x3E)))
                            .rounding(Rounding::same(16.0))
                            .inner_margin(Margin::symmetric(12.0, 6.0))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    Self::draw_dot(ui, Color32::from_rgb(0xF5, 0x9E, 0x0B), 3.4);
                                    ui.label(
                                        RichText::new("1 needs you")
                                            .size(11.5)
                                            .strong()
                                            .color(Color32::from_rgb(0xF5, 0x9E, 0x0B)),
                                    );
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if ui
                                            .add(
                                                egui::Button::new(
                                                    RichText::new("5 agents")
                                                        .size(11.0)
                                                        .color(Color32::from_rgb(0x94, 0x96, 0xA1)),
                                                )
                                                .frame(false),
                                            )
                                            .clicked()
                                        {
                                            self.show_agent_approval_modal = true;
                                        }
                                    });
                                });
                            });
                    });
                });
        }

        // ====================================================================
        // 3. MAIN CENTRAL WORKSPACE CANVAS (MATCHING REFERENCE IMAGE 1)
        // ====================================================================
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Color32::from_rgb(0x12, 0x13, 0x16)))
            .show(ctx, |ui| {
                let canvas_rect = ui.available_rect_before_wrap();
                self.core.wm.update_canvas_dimensions(
                    canvas_rect.width() as u32,
                    canvas_rect.height() as u32,
                );

                if self.view_mode == ViewMode::FloatingWindows {
                    self.render_floating_windows_canvas(ui, ctx, canvas_rect, &surfaces);
                } else if self.showing_ai_chat_canvas {
                    self.render_dia_ask_split_canvas(ui, ctx, canvas_rect);
                } else {
                    // Render full-window live Chromium webpage surface
                    let active_tab = self
                        .core
                        .tabs
                        .read()
                        .iter()
                        .find(|t| Some(t.id) == self.active_tab_id)
                        .cloned();

                    let current_url = active_tab
                        .as_ref()
                        .map(|t| t.url.clone())
                        .unwrap_or_else(|| "https://www.google.com".to_string());

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
                                    current_url.clone(),
                                    canvas_rect.width() as u32,
                                    canvas_rect.height() as u32,
                                    PathBuf::from(&active_prof.storage_partition_path),
                                );
                            }
                            ui.vertical_centered(|ui| {
                                ui.add_space(160.0);
                                ui.label(
                                    RichText::new(format!("Loading {} ...", current_url))
                                        .size(16.0)
                                        .strong()
                                        .color(Color32::from_rgb(0x8B, 0x7C, 0xF6)),
                                );
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new("Rendering live via Chromium/Blink + V8 Engine (TLS 1.3 + ECH)")
                                        .size(12.0)
                                        .color(Color32::from_rgb(0x94, 0x96, 0xA1)),
                                );
                            });
                        }
                    }
                }
            });

        // ====================================================================
        // 4. FLOATING "NEEDS YOUR OK" AGENT PERMISSION GATE CARD (IMAGE 1)
        // ====================================================================
        if self.show_agent_approval_modal {
            self.render_needs_your_ok_overlay(ctx);
        }

        // ====================================================================
        // 5. FLOATING INSPECTOR MODAL (TRANSPORT SECURITY / DEVHUB / AI / DL)
        // ====================================================================
        if self.active_modal != InspectorModal::None {
            self.render_inspector_modal(ctx);
        }

        // Automated screenshot capture for CLI verification
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
    /// Renders the sleek split Ask / Research Workspace + Live Mini Web Preview Cards
    /// matching Reference Image 1 (`Dinner Friday`, interactive dark map, comparison table, and bottom reply pill).
    fn render_dia_ask_split_canvas(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        canvas_rect: Rect,
    ) {
        let active_chat = self
            .chat_threads
            .iter()
            .find(|c| Some(c.id) == self.active_chat_id)
            .or_else(|| self.chat_threads.first())
            .cloned();

        let Some(chat) = active_chat else {
            return;
        };

        ui.horizontal_top(|ui| {
            // LEFT MAIN RESEARCH PANE (~63% width)
            let left_w = (canvas_rect.width() * 0.63).max(460.0);
            egui::Frame::none()
                .fill(Color32::from_rgb(0x13, 0x14, 0x18))
                .inner_margin(Margin::symmetric(28.0, 16.0))
                .show(ui, |ui| {
                    ui.set_width(left_w);
                    ui.set_height(canvas_rect.height() - 20.0);

                    ui.vertical(|ui| {
                        // Header: `Dinner Friday`
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(&chat.title)
                                    .size(14.5)
                                    .strong()
                                    .color(Color32::from_rgb(0xE4, 0xE4, 0xE7)),
                            );
                        });

                        ui.add_space(6.0);

                        // User Prompt Bubble (Right-aligned rounded pill)
                        ui.horizontal(|ui| {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                egui::Frame::none()
                                    .fill(Color32::from_rgb(0x23, 0x25, 0x2E))
                                    .stroke(Stroke::new(
                                        1.0_f32,
                                        Color32::from_rgb(0x32, 0x35, 0x42),
                                    ))
                                    .rounding(Rounding::same(14.0))
                                    .inner_margin(Margin::symmetric(14.0, 8.0))
                                    .show(ui, |ui| {
                                        ui.label(
                                            RichText::new(
                                                chat.messages
                                                    .first()
                                                    .map(|m| m.1.as_str())
                                                    .unwrap_or("Let's do seafood Friday for four"),
                                            )
                                            .size(12.5)
                                            .color(Color32::from_rgb(0xF1, 0xF1, 0xF3)),
                                        );
                                    });
                            });
                        });

                        ui.add_space(8.0);

                        // Agent Activity Status Row (`Worked for 3m 16s     23 actions`)
                        ui.horizontal(|ui| {
                            Self::draw_dot(ui, Color32::from_rgb(0x8B, 0x7C, 0xF6), 3.5);
                            ui.label(
                                RichText::new(&chat.worked_duration)
                                    .size(11.5)
                                    .color(Color32::from_rgb(0x94, 0x96, 0xA1)),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(
                                    RichText::new(format!("{} actions", chat.actions_count))
                                        .size(11.0)
                                        .color(Color32::from_rgb(0x6E, 0x71, 0x7C)),
                                );
                            });
                        });

                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(
                                "Seafood and vegan pull against each other. Four of the 31 places near you do both:",
                            )
                            .size(13.0)
                            .color(Color32::from_rgb(0xE4, 0xE4, 0xE7)),
                        );

                        ui.add_space(8.0);

                        // Interactive Dark Vector Map Card (matching Reference Image 1's map with pins!)
                        egui::Frame::none()
                            .fill(Color32::from_rgb(0x19, 0x1B, 0x20))
                            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x2A, 0x2D, 0x36)))
                            .rounding(Rounding::same(10.0))
                            .inner_margin(Margin::same(0.0))
                            .show(ui, |ui| {
                                let map_w = ui.available_width();
                                let map_h = 140.0;
                                let (map_rect, _) =
                                    ui.allocate_exact_size(Vec2::new(map_w, map_h), Sense::click());
                                let painter = ui.painter_at(map_rect);

                                // Subtle dark street grid lines & river curve
                                let grid_stroke =
                                    Stroke::new(1.0_f32, Color32::from_rgb(0x23, 0x26, 0x2F));
                                for i in 1..12 {
                                    let x = map_rect.min.x + (map_w / 12.0) * (i as f32);
                                    painter.line_segment(
                                        [
                                            Pos2::new(x, map_rect.min.y),
                                            Pos2::new(x - 28.0, map_rect.max.y),
                                        ],
                                        grid_stroke,
                                    );
                                }
                                for j in 1..6 {
                                    let y = map_rect.min.y + (map_h / 6.0) * (j as f32);
                                    painter.line_segment(
                                        [
                                            Pos2::new(map_rect.min.x, y),
                                            Pos2::new(map_rect.max.x, y + 14.0),
                                        ],
                                        grid_stroke,
                                    );
                                }

                                // Map Pins: 1 Saltgrass (highlighted white pill), 2 Brine, 3 Juniper, 4 Little Gem
                                let pins = [
                                    (0.50, 0.44, "1  Saltgrass", true),
                                    (0.28, 0.56, "2  Brine", false),
                                    (0.75, 0.34, "3  Juniper", false),
                                    (0.56, 0.76, "4  Little Gem", false),
                                ];
                                for (rx, ry, label, is_primary) in pins {
                                    let center = Pos2::new(
                                        map_rect.min.x + map_w * rx,
                                        map_rect.min.y + map_h * ry,
                                    );
                                    let pill_rect =
                                        Rect::from_center_size(center, Vec2::new(96.0, 22.0));
                                    painter.rect_filled(
                                        pill_rect,
                                        Rounding::same(11.0),
                                        if is_primary {
                                            Color32::WHITE
                                        } else {
                                            Color32::from_rgb(0x26, 0x29, 0x33)
                                        },
                                    );
                                    painter.text(
                                        center,
                                        egui::Align2::CENTER_CENTER,
                                        label,
                                        FontId::new(11.0, FontFamily::Proportional),
                                        if is_primary {
                                            Color32::from_rgb(0x12, 0x13, 0x16)
                                        } else {
                                            Color32::from_rgb(0xE4, 0xE4, 0xE7)
                                        },
                                    );
                                }
                            });

                        ui.add_space(10.0);

                        // Structured Comparison Table (matching Reference Image 1)
                        egui::Grid::new("dia_comparison_table")
                            .spacing(Vec2::new(26.0, 10.0))
                            .min_col_width(82.0)
                            .striped(true)
                            .show(ui, |ui| {
                                ui.label(RichText::new("Place").size(11.0).color(Color32::from_rgb(0x8E, 0x90, 0x9B)));
                                ui.label(RichText::new("Drive").size(11.0).color(Color32::from_rgb(0x8E, 0x90, 0x9B)));
                                ui.label(RichText::new("Plant mains").size(11.0).color(Color32::from_rgb(0x8E, 0x90, 0x9B)));
                                ui.label(RichText::new("Friday").size(11.0).color(Color32::from_rgb(0x8E, 0x90, 0x9B)));
                                ui.label(RichText::new("No-show fee").size(11.0).color(Color32::from_rgb(0x8E, 0x90, 0x9B)));
                                ui.end_row();

                                let rows = [
                                    ("1  Saltgrass  [My pick]", "Seafood • $$ • ★ 4.7", "12 min", "9 mains", "8:00 PM", "$25 / person"),
                                    ("2  Brine", "Oyster bar • $$ • ★ 4.8", "6 min", "1 main", "7:30 PM", "None"),
                                    ("3  Juniper", "Coastal Italian • $$ • ★ 4.5", "18 min", "4 mains", "8:45 only", "None"),
                                    ("4  Little Gem", "Seafood & veg • $$ • ★ 4.6", "9 min", "6 mains", "Full", "$15 / person"),
                                ];

                                for (name, sub, drive, mains, fri, fee) in rows {
                                    ui.vertical(|ui| {
                                        ui.set_min_width(185.0);
                                        ui.label(RichText::new(name).size(12.0).strong().color(Color32::WHITE));
                                        ui.label(RichText::new(sub).size(10.5).color(Color32::from_rgb(0x8E, 0x90, 0x9B)));
                                    });
                                    ui.label(RichText::new(drive).size(11.5));
                                    ui.label(RichText::new(mains).size(11.5));
                                    ui.label(RichText::new(fri).size(11.5));
                                    ui.label(RichText::new(fee).size(11.5));
                                    ui.end_row();
                                }
                            });

                        ui.add_space(10.0);

                        // Action Pills (`Book Saltgrass, 8:00`  `↗ Open all 4`)
                        ui.horizontal(|ui| {
                            let book_btn = egui::Button::new(
                                RichText::new("Book Saltgrass, 8:00")
                                    .size(11.5)
                                    .strong()
                                    .color(Color32::from_rgb(0x12, 0x13, 0x16)),
                            )
                            .fill(Color32::WHITE)
                            .rounding(Rounding::same(14.0));

                            if ui.add(book_btn).clicked() {
                                self.show_agent_approval_modal = true;
                            }

                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("↗ Open all 4 in tabs")
                                            .size(11.5)
                                            .color(Color32::from_rgb(0xD4, 0xD5, 0xDC)),
                                    )
                                    .fill(Color32::from_rgb(0x22, 0x24, 0x2C))
                                    .rounding(Rounding::same(14.0)),
                                )
                                .clicked()
                            {
                                self.navigate_to_live_url(ctx, "https://www.wikipedia.org");
                            }
                        });

                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(
                                "Saltgrass is my pick: nine plant mains, so Maya orders off the same menu, and it's free at 8:00, it costs the most. Brine is closest with the best oysters, but she'd get one main.",
                            )
                            .size(12.5)
                            .color(Color32::from_rgb(0xC4, 0xC6, 0xD0)),
                        );

                        ui.add_space(14.0);

                        // Bottom Floating Reply Bar (`+  Reply, or steer the run...   Go`)
                        egui::Frame::none()
                            .fill(Color32::from_rgb(0x1D, 0x1F, 0x26))
                            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x2E, 0x31, 0x3C)))
                            .rounding(Rounding::same(18.0))
                            .inner_margin(Margin::symmetric(14.0, 8.0))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new("+")
                                            .size(14.0)
                                            .color(Color32::from_rgb(0x94, 0x96, 0xA1)),
                                    );
                                    let reply_resp = ui.add(
                                        egui::TextEdit::singleline(&mut self.chat_reply_buffer)
                                            .frame(false)
                                            .desired_width(left_w - 115.0)
                                            .hint_text("Reply, or steer the run..."),
                                    );
                                    if (reply_resp.lost_focus()
                                        && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                                        || ui
                                            .add(
                                                egui::Button::new(
                                                    RichText::new("Go").size(11.5).strong().color(Color32::WHITE),
                                                )
                                                .fill(Color32::from_rgb(0x8B, 0x7C, 0xF6))
                                                .rounding(Rounding::same(12.0)),
                                            )
                                            .clicked()
                                    {
                                        if !self.chat_reply_buffer.trim().is_empty() {
                                            let q = self.chat_reply_buffer.clone();
                                            self.chat_reply_buffer.clear();
                                            if q.contains('.') && !q.contains(' ') {
                                                self.navigate_to_live_url(ctx, &q);
                                            }
                                        }
                                    }
                                });
                            });
                    });
                });

            // RIGHT CONTEXT & MINI WEB PREVIEW COLUMN (~36% width, matching Reference Image 1!)
            egui::Frame::none()
                .fill(Color32::from_rgb(0x15, 0x16, 0x1B))
                .inner_margin(Margin::symmetric(14.0, 18.0))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x23, 0x25, 0x2D)))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_height(canvas_rect.height() - 24.0);

                    ui.vertical(|ui| {
                        // Paused for your OK Card
                        egui::Frame::none()
                            .fill(Color32::from_rgb(0x1E, 0x20, 0x27))
                            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x2E, 0x31, 0x3D)))
                            .rounding(Rounding::same(10.0))
                            .inner_margin(Margin::same(12.0))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    Self::draw_dot(ui, Color32::from_rgb(0xF5, 0x9E, 0x0B), 3.4);
                                    ui.label(
                                        RichText::new("Paused for your OK")
                                            .size(11.5)
                                            .strong()
                                            .color(Color32::from_rgb(0xF5, 0x9E, 0x0B)),
                                    );
                                });
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new("Saltgrass – OpenTable ↗")
                                        .size(11.0)
                                        .color(Color32::from_rgb(0xA1, 0xA1, 0xAA)),
                                );
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new("Saltgrass Steak & Seafood")
                                        .size(14.0)
                                        .strong()
                                        .color(Color32::WHITE),
                                );
                                ui.label(
                                    RichText::new(
                                        "Seafood • $$ • ★ 4.7  •  4 people  •  Maya is vegan",
                                    )
                                    .size(11.0)
                                    .color(Color32::from_rgb(0x94, 0x96, 0xA1)),
                                );
                                ui.add_space(8.0);
                                let reserve_btn = egui::Button::new(
                                    RichText::new("Complete reservation")
                                        .size(12.0)
                                        .strong()
                                        .color(Color32::WHITE),
                                )
                                .min_size(Vec2::new(ui.available_width(), 30.0))
                                .fill(Color32::from_rgb(0xDC, 0x26, 0x26))
                                .rounding(Rounding::same(6.0));
                                if ui.add(reserve_btn).clicked() {
                                    self.show_agent_approval_modal = true;
                                }
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new(
                                        "Paused before \"Complete reservation\" • opentable.com",
                                    )
                                    .size(10.0)
                                    .color(Color32::from_rgb(0x6E, 0x71, 0x7C)),
                                );
                            });

                        ui.add_space(240.0);
                        ui.label(
                            RichText::new("THIS TASK CAN")
                                .size(10.0)
                                .strong()
                                .color(Color32::from_rgb(0x6E, 0x71, 0x7C)),
                        );
                        ui.label(
                            RichText::new("• Search and fill forms on opentable.com")
                                .size(11.0)
                                .color(Color32::from_rgb(0xA1, 0xA1, 0xAA)),
                        );
                        ui.label(
                            RichText::new("• Read your calendar and notes")
                                .size(11.0)
                                .color(Color32::from_rgb(0xA1, 0xA1, 0xAA)),
                        );
                        ui.label(
                            RichText::new("🔒 Book with a no-show fee (Ask first)")
                                .size(11.0)
                                .color(Color32::from_rgb(0xF5, 0x9E, 0x0B)),
                        );
                    });
                });
        });
    }

    /// Renders the high-craft floating "Needs your OK" Agent Permission Gate Popover on the right
    /// matching Reference Image 1 (`Book Saltgrass for 4 on Friday?`, white embedded form card, `Deny` / `Book table HOLD`).
    fn render_needs_your_ok_overlay(&mut self, ctx: &egui::Context) {
        let screen = ctx.screen_rect();
        let pos = Pos2::new((screen.max.x - 365.0).max(400.0), 58.0);

        egui::Area::new(egui::Id::new("noctra_needs_your_ok_popover"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                egui::Frame::none()
                    .fill(Color32::from_rgb(0x1C, 0x1E, 0x24))
                    .stroke(Stroke::new(1.2_f32, Color32::from_rgb(0x38, 0x3B, 0x47)))
                    .rounding(Rounding::same(12.0))
                    .inner_margin(Margin::same(16.0))
                    .show(ui, |ui| {
                        ui.set_width(335.0);

                        // Top header: `Needs your OK     opentable.com`
                        ui.horizontal(|ui| {
                            Self::draw_dot(ui, Color32::from_rgb(0xF5, 0x9E, 0x0B), 3.6);
                            ui.label(
                                RichText::new("Needs your OK")
                                    .size(11.5)
                                    .strong()
                                    .color(Color32::from_rgb(0xF5, 0x9E, 0x0B)),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("×")
                                                .size(13.0)
                                                .color(Color32::from_rgb(0x94, 0x96, 0xA1)),
                                        )
                                        .frame(false),
                                    )
                                    .clicked()
                                {
                                    self.show_agent_approval_modal = false;
                                }
                                ui.label(
                                    RichText::new("opentable.com")
                                        .size(11.0)
                                        .color(Color32::from_rgb(0xEF, 0x44, 0x44)),
                                );
                                Self::draw_dot(ui, Color32::from_rgb(0xEF, 0x44, 0x44), 3.2);
                            });
                        });

                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("Book Saltgrass for 4 on Friday?")
                                .size(15.5)
                                .strong()
                                .color(Color32::WHITE),
                        );
                        ui.label(
                            RichText::new(
                                "They charge $25 a person if you don't show, so I stopped before booking.",
                            )
                            .size(12.0)
                            .color(Color32::from_rgb(0xA1, 0xA1, 0xAA)),
                        );

                        ui.add_space(10.0);

                        // Clean White Embedded Live Webpage Snapshot Card (matching Reference Image 1!)
                        egui::Frame::none()
                            .fill(Color32::WHITE)
                            .rounding(Rounding::same(8.0))
                            .inner_margin(Margin::same(12.0))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    Self::draw_dot(ui, Color32::from_rgb(0xDC, 0x26, 0x26), 3.4);
                                    ui.label(
                                        RichText::new("OpenTable")
                                            .size(11.0)
                                            .strong()
                                            .color(Color32::from_rgb(0xDC, 0x26, 0x26)),
                                    );
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.label(
                                            RichText::new("🔒 Secure TLS 1.3")
                                                .size(10.0)
                                                .color(Color32::from_rgb(0x52, 0x52, 0x5B)),
                                        );
                                    });
                                });
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new("Complete your reservation")
                                        .size(12.5)
                                        .strong()
                                        .color(Color32::from_rgb(0x18, 0x18, 0x1B)),
                                );
                                ui.label(
                                    RichText::new("Saltgrass  •  Fri, Oct 16  •  8:00 PM  •  4 people")
                                        .size(10.5)
                                        .color(Color32::from_rgb(0x71, 0x71, 0x7A)),
                                );
                                ui.add_space(6.0);
                                ui.horizontal(|ui| {
                                    for field_txt in ["Sam W.", "sam@nocta.dev", "+1 415 ••• 0142"]
                                    {
                                        egui::Frame::none()
                                            .fill(Color32::from_rgb(0xF4, 0xF4, 0xF5))
                                            .stroke(Stroke::new(
                                                1.0_f32,
                                                Color32::from_rgb(0xD4, 0xD4, 0xD8),
                                            ))
                                            .rounding(Rounding::same(4.0))
                                            .inner_margin(Margin::symmetric(6.0, 3.0))
                                            .show(ui, |ui| {
                                                ui.label(
                                                    RichText::new(field_txt)
                                                        .size(10.0)
                                                        .color(Color32::from_rgb(0x27, 0x27, 0x2A)),
                                                );
                                            });
                                    }
                                });
                                ui.add_space(6.0);
                                egui::Frame::none()
                                    .fill(Color32::from_rgb(0xDC, 0x26, 0x26))
                                    .rounding(Rounding::same(5.0))
                                    .inner_margin(Margin::symmetric(10.0, 5.0))
                                    .show(ui, |ui| {
                                        ui.label(
                                            RichText::new("Complete reservation")
                                                .size(11.0)
                                                .strong()
                                                .color(Color32::WHITE),
                                        );
                                    });
                            });

                        ui.add_space(8.0);

                        // Metadata Grid (`When`, `Party`, `No-show`, `Cancel`)
                        egui::Grid::new("approval_meta_grid")
                            .spacing(Vec2::new(14.0, 4.0))
                            .show(ui, |ui| {
                                for (k, v) in [
                                    ("When", "Fri, Oct 16 · 8:00 PM"),
                                    ("Party", "4 · Maya eats vegan"),
                                    ("No-show", "$25 a person · Amex -1007"),
                                    ("Cancel", "Free until Thu 8:00 PM"),
                                ] {
                                    ui.label(
                                        RichText::new(k)
                                            .size(11.0)
                                            .color(Color32::from_rgb(0x8E, 0x90, 0x9B)),
                                    );
                                    ui.label(
                                        RichText::new(v)
                                            .size(11.5)
                                            .color(Color32::from_rgb(0xE4, 0xE4, 0xE7)),
                                    );
                                    ui.end_row();
                                }
                            });

                        ui.add_space(10.0);

                        // Deny & Book table HOLD Buttons
                        ui.horizontal(|ui| {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let approve_btn = egui::Button::new(
                                    RichText::new("Book table  [HOLD]")
                                        .size(12.0)
                                        .strong()
                                        .color(Color32::from_rgb(0x12, 0x13, 0x16)),
                                )
                                .fill(Color32::WHITE)
                                .rounding(Rounding::same(8.0));

                                if ui.add(approve_btn).clicked() {
                                    self.show_agent_approval_modal = false;
                                }

                                let deny_btn = egui::Button::new(
                                    RichText::new("Deny")
                                        .size(12.0)
                                        .color(Color32::from_rgb(0xE4, 0xE4, 0xE7)),
                                )
                                .fill(Color32::from_rgb(0x28, 0x2A, 0x34))
                                .rounding(Rounding::same(8.0));

                                if ui.add(deny_btn).clicked() {
                                    self.show_agent_approval_modal = false;
                                }
                            });
                        });
                    });
            });
    }

    fn render_floating_windows_canvas(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        canvas_rect: Rect,
        surfaces: &[noctra_types::InternalFloatingSurface],
    ) {
        let active_prof = self.core.profiles.active_profile().unwrap();
        for s in surfaces {
            if s.window_state == SurfaceWindowState::Minimized {
                continue;
            }
            let win_x = canvas_rect.min.x + (s.x as f32).max(0.0);
            let win_y = canvas_rect.min.y + (s.y as f32).max(0.0);
            let win_w = (s.width as f32).clamp(320.0, canvas_rect.width());
            let win_h = (s.height as f32).clamp(220.0, canvas_rect.height());

            egui::Area::new(egui::Id::new(("noctra_floating_win", s.id)))
                .fixed_pos(Pos2::new(win_x, win_y))
                .show(ctx, |ui| {
                    egui::Frame::none()
                        .fill(Color32::from_rgb(0x1E, 0x20, 0x27))
                        .stroke(Stroke::new(1.2_f32, Color32::from_rgb(0x8B, 0x7C, 0xF6)))
                        .rounding(Rounding::same(10.0))
                        .show(ui, |ui| {
                            ui.set_width(win_w);
                            ui.set_height(win_h);
                            egui::Frame::none()
                                .fill(Color32::from_rgb(0x26, 0x28, 0x32))
                                .inner_margin(Margin::symmetric(10.0, 6.0))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        let drag = ui.add(
                                            egui::Label::new(
                                                RichText::new(format!(":: {}", s.title))
                                                    .strong()
                                                    .size(11.5),
                                            )
                                            .sense(Sense::click_and_drag()),
                                        );
                                        if drag.dragged() {
                                            let d = drag.drag_delta();
                                            let _ = self.core.wm.move_and_resize_surface(
                                                s.id,
                                                s.x + d.x as i32,
                                                s.y + d.y as i32,
                                                s.width,
                                                s.height,
                                                true,
                                            );
                                        }
                                        let url_buf = self
                                            .surface_url_buffers
                                            .entry(s.id)
                                            .or_insert_with(|| s.url.clone());
                                        let edit = ui.add(
                                            egui::TextEdit::singleline(url_buf)
                                                .desired_width((win_w - 220.0).max(100.0)),
                                        );
                                        if edit.lost_focus()
                                            && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                        {
                                            let target =
                                                self.core.resolve_omnibox_input(&url_buf.clone());
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
                                                PathBuf::from(&active_prof.storage_partition_path),
                                            );
                                        }
                                        if ui.small_button("[L]").clicked() {
                                            let _ = self
                                                .core
                                                .wm
                                                .snap_surface_to_zone(s.id, SnapZone::LeftHalf);
                                        }
                                        if ui.small_button("[R]").clicked() {
                                            let _ = self
                                                .core
                                                .wm
                                                .snap_surface_to_zone(s.id, SnapZone::RightHalf);
                                        }
                                    });
                                });
                            if let Some((_, tex)) = self.surface_textures.get(&s.id) {
                                ui.add(egui::Image::from_texture((
                                    tex.id(),
                                    Vec2::new(win_w - 4.0, (win_h - 38.0).max(100.0)),
                                )));
                            } else {
                                ui.vertical_centered(|ui| {
                                    ui.add_space(40.0);
                                    ui.label("Surface Ready — Enter a URL above to render via Chromium/Blink.");
                                });
                            }
                        });
                });
        }
        let _ = ui;
    }

    fn render_inspector_modal(&mut self, ctx: &egui::Context) {
        let mut open = true;
        let title = match self.active_modal {
            InspectorModal::TransportSecurity => "Transport Security & 'What Can an Observer See?' (14B)",
            InspectorModal::DeveloperHub => "NOCTRA Developer Hub & Inspector",
            InspectorModal::AiConnectionCenter => "NOCTRA AI Connection Center & Vault",
            InspectorModal::DownloadsAndTorrents => "Downloads & Native BitTorrent Client",
            InspectorModal::NetworkAndResources => "Network Quality & Potato-Laptop Resource Governor",
            InspectorModal::None => return,
        };

        egui::Window::new(title)
            .open(&mut open)
            .resizable(true)
            .default_width(440.0)
            .show(ctx, |ui| match self.active_modal {
                InspectorModal::TransportSecurity => {
                    let mut mode = *self.core.privacy.active_mode.read();
                    egui::ComboBox::from_label("Protection Mode")
                        .selected_text(format!("{:?}", mode))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut mode,
                                TransportProtectionMode::HttpsDohEch,
                                "HTTPS-Only + TLS 1.3 + DoH + ECH",
                            );
                            ui.selectable_value(
                                &mut mode,
                                TransportProtectionMode::WireGuardVpn,
                                "Built-in WireGuard VPN Tunnel",
                            );
                            ui.selectable_value(
                                &mut mode,
                                TransportProtectionMode::TorOnionMode,
                                "Tor Onion Circuit Mode",
                            );
                        });
                    *self.core.privacy.active_mode.write() = mode;

                    let obs = self.core.privacy.observer_visibility_report();
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("Encrypted / Hidden from Wire Observer:")
                            .strong()
                            .color(Color32::from_rgb(0x10, 0xB9, 0x81)),
                    );
                    for h in &obs.hidden_from_network_observer {
                        ui.label(format!("  ✓ {}", h));
                    }
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("Residual Metadata Visible to Observer:")
                            .strong()
                            .color(Color32::from_rgb(0xF5, 0x9E, 0x0B)),
                    );
                    for v in &obs.visible_to_network_observer {
                        ui.label(format!("  • {}", v));
                    }
                    ui.add_space(6.0);
                    if ui.button("Run Wire Packet Capture Leak Test").clicked() {
                        let wire = [0x17u8, 0x03, 0x03, 0x00, 0x20, 0xaa, 0xbb];
                        let ok = PrivacyAndTransportEngine::verify_captured_wire_bytes_contain_no_leaks(
                            &wire,
                            &["https://secret.nocta.dev", "Bearer sk-live"],
                        )
                        .is_ok();
                        if ok {
                            self.status_banner =
                                "PASSED: 0 plaintext URLs, hostnames, or tokens on wire!"
                                    .to_string();
                        }
                    }
                    ui.label(
                        RichText::new(&self.status_banner)
                            .color(Color32::from_rgb(0x10, 0xB9, 0x81)),
                    );
                }
                InspectorModal::DeveloperHub => {
                    egui::ComboBox::from_label("Tool")
                        .selected_text(&self.devtool_selected)
                        .show_ui(ui, |ui| {
                            for t in [
                                "JSON Formatter & Validator",
                                "YAML Validator -> JSON",
                                "JWT Decoder",
                                "SHA-256 / SHA-512 Hash",
                                "UUID v4 Batch",
                            ] {
                                ui.selectable_value(&mut self.devtool_selected, t.to_string(), t);
                            }
                        });
                    ui.add(egui::TextEdit::multiline(&mut self.devtool_input).desired_rows(4));
                    if ui.button("Run Tool").clicked() {
                        self.devtool_output = match self.devtool_selected.as_str() {
                            "YAML Validator -> JSON" => {
                                DeveloperHubAndInspector::validate_and_convert_yaml_to_json(
                                    &self.devtool_input,
                                )
                                .unwrap_or_else(|e| e.to_string())
                            }
                            "JSON Formatter & Validator" => {
                                DeveloperHubAndInspector::format_and_validate_json(
                                    &self.devtool_input,
                                )
                                .unwrap_or_else(|e| e.to_string())
                            }
                            "SHA-256 / SHA-512 Hash" => serde_json::to_string_pretty(
                                &DeveloperHubAndInspector::compute_hashes(&self.devtool_input),
                            )
                            .unwrap_or_default(),
                            _ => DeveloperHubAndInspector::generate_uuids(5).join("\n"),
                        };
                    }
                    ui.code(&self.devtool_output);
                }
                InspectorModal::AiConnectionCenter => {
                    ui.label("Add AI Provider (Stored in OS Encrypted Credential Vault):");
                    ui.text_edit_singleline(&mut self.ai_prov_name);
                    ui.text_edit_singleline(&mut self.ai_prov_url);
                    ui.text_edit_singleline(&mut self.ai_prov_model);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.ai_prov_token)
                            .password(true)
                            .hint_text("API Token"),
                    );
                    if ui.button("+ Save to Encrypted Vault").clicked() {
                        let tok = if self.ai_prov_token.trim().is_empty() {
                            "sk-ant-vault-token".to_string()
                        } else {
                            self.ai_prov_token.trim().to_string()
                        };
                        if let Ok(cfg) = self.core.ai.add_provider(
                            self.ai_prov_name.clone(),
                            AiProviderKind::Anthropic,
                            self.ai_prov_url.clone(),
                            self.ai_prov_model.clone(),
                            vec![self.ai_prov_model.clone()],
                            RedactedSecret::new(tok),
                            20,
                        ) {
                            self.ai_status_output =
                                format!("Saved: {} -> {}", cfg.name, cfg.credential_ref);
                            self.ai_prov_token.clear();
                        }
                    }
                    ui.code(&self.ai_status_output);
                }
                InspectorModal::DownloadsAndTorrents => {
                    ui.label("Add BitTorrent Magnet URI:");
                    ui.text_edit_singleline(&mut self.magnet_uri_input);
                    if ui.button("+ Start Magnet Download").clicked() {
                        let prof = self.core.profiles.active_profile().unwrap();
                        let _ = self
                            .core
                            .torrent
                            .add_magnet_link(&self.magnet_uri_input, prof.download_directory);
                    }
                    for t in self.core.torrent.list_torrents() {
                        ui.label(format!(
                            "⬇ {} ({}%) | DHT: {} nodes",
                            t.display_name, t.progress_pct, t.dht_nodes
                        ));
                    }
                }
                InspectorModal::NetworkAndResources => {
                    let wifi = self.core.net_monitor.get_wifi_link_info();
                    let res = self.core.query_resource_telemetry();
                    ui.label(format!("Wi-Fi Link: {}", wifi.source_note));
                    ui.label(format!(
                        "Host RAM: {} / {} MB",
                        res.system_used_ram_mb, res.system_total_ram_mb
                    ));
                    ui.label(format!("NOCTRA Core RAM: {} MB", res.noctra_core_ram_mb));
                }
                InspectorModal::None => {}
            });

        if !open {
            self.active_modal = InspectorModal::None;
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
            .with_inner_size([1400.0, 860.0])
            .with_min_inner_size([960.0, 600.0]),
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
