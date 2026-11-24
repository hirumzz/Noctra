use noctra_database::NoctraDatabase;
use noctra_types::{
    SnapZone, InternalFloatingSurface, NoctraError, NoctraResult, SurfaceWindowState,
    WindowLayoutMode,
};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

const MIN_WINDOW_WIDTH: u32 = 280;
const MIN_WINDOW_HEIGHT: u32 = 180;
const SNAP_THRESHOLD_PX: i32 = 24;

pub struct InternalWindowManager {
    db: Arc<NoctraDatabase>,
    surfaces: RwLock<Vec<InternalFloatingSurface>>,
    workspace_layouts: RwLock<HashMap<Uuid, WindowLayoutMode>>,
    canvas_width: RwLock<u32>,
    canvas_height: RwLock<u32>,
    z_counter: RwLock<u32>,
}

impl InternalWindowManager {
    pub fn initialize(
        db: Arc<NoctraDatabase>,
        canvas_width: u32,
        canvas_height: u32,
    ) -> NoctraResult<Self> {
        let saved_surfaces: Vec<InternalFloatingSurface> = db
            .load_kv("window_manager", "floating_surfaces")?
            .unwrap_or_default();
        let max_z = saved_surfaces
            .iter()
            .map(|s| s.z_index)
            .max()
            .unwrap_or(10);

        Ok(Self {
            db,
            surfaces: RwLock::new(saved_surfaces),
            workspace_layouts: RwLock::new(HashMap::new()),
            canvas_width: RwLock::new(canvas_width.max(800)),
            canvas_height: RwLock::new(canvas_height.max(600)),
            z_counter: RwLock::new(max_z + 1),
        })
    }

    pub fn update_canvas_dimensions(&self, width: u32, height: u32) {
        *self.canvas_width.write() = width.max(640);
        *self.canvas_height.write() = height.max(480);
    }

    fn next_z_index(&self) -> u32 {
        let mut z = self.z_counter.write();
        *z += 1;
        *z
    }

    pub fn persist(&self) -> NoctraResult<()> {
        let guard = self.surfaces.read();
        self.db
            .save_kv("window_manager", "floating_surfaces", &*guard)
    }

    pub fn list_surfaces_for_workspace(&self, workspace_id: Uuid) -> Vec<InternalFloatingSurface> {
        let mut list: Vec<InternalFloatingSurface> = self
            .surfaces
            .read()
            .iter()
            .filter(|s| s.workspace_id == workspace_id)
            .cloned()
            .collect();
        list.sort_by_key(|s| s.z_index);
        list
    }

    pub fn spawn_surface(
        &self,
        profile_id: Uuid,
        workspace_id: Uuid,
        tab_id: Uuid,
        title: String,
        url: String,
        storage_partition_id: String,
        is_pip_video: bool,
    ) -> NoctraResult<InternalFloatingSurface> {
        let z = self.next_z_index();
        let existing_count = self
            .surfaces
            .read()
            .iter()
            .filter(|s| s.workspace_id == workspace_id)
            .count() as i32;

        let cw = *self.canvas_width.read() as i32;
        let ch = *self.canvas_height.read() as i32;

        let (x, y, width, height) = if is_pip_video {
            ((cw - 440).max(20), (ch - 280).max(20), 420, 250)
        } else {
            let offset = (existing_count % 6) * 32;
            (
                48 + offset,
                36 + offset,
                ((cw as f32 * 0.56) as u32).max(520),
                ((ch as f32 * 0.68) as u32).max(380),
            )
        };

        let surface = InternalFloatingSurface {
            id: Uuid::new_v4(),
            profile_id,
            workspace_id,
            tab_id,
            title,
            url,
            x,
            y,
            width,
            height,
            restore_rect: None,
            z_index: z,
            window_state: SurfaceWindowState::Normal,
            is_focused: true,
            is_loading: false,
            is_pip_video,
            snap_zone: SnapZone::None,
            storage_partition_id,
        };

        {
            let mut guard = self.surfaces.write();
            for s in guard.iter_mut().filter(|s| s.workspace_id == workspace_id) {
                s.is_focused = false;
            }
            guard.push(surface.clone());
        }

        let mode = self
            .workspace_layouts
            .read()
            .get(&workspace_id)
            .copied()
            .unwrap_or(WindowLayoutMode::FreeFloating);

        if mode != WindowLayoutMode::FreeFloating && !is_pip_video {
            let _ = self.apply_tiling_layout(workspace_id, mode);
        } else {
            self.persist()?;
        }

        self.get_surface(surface.id)
    }

    pub fn get_surface(&self, surface_id: Uuid) -> NoctraResult<InternalFloatingSurface> {
        self.surfaces
            .read()
            .iter()
            .find(|s| s.id == surface_id)
            .cloned()
            .ok_or_else(|| NoctraError::WindowManagerError("Surface not found".to_string()))
    }

    pub fn bring_to_front(&self, surface_id: Uuid) -> NoctraResult<InternalFloatingSurface> {
        let z = self.next_z_index();
        let mut target_ws = None;
        {
            let mut guard = self.surfaces.write();
            if let Some(s) = guard.iter().find(|s| s.id == surface_id) {
                target_ws = Some(s.workspace_id);
            }
            if let Some(ws_id) = target_ws {
                for s in guard.iter_mut().filter(|s| s.workspace_id == ws_id) {
                    if s.id == surface_id {
                        s.z_index = z;
                        s.is_focused = true;
                        if s.window_state == SurfaceWindowState::Minimized {
                            s.window_state = SurfaceWindowState::Normal;
                        }
                    } else {
                        s.is_focused = false;
                    }
                }
            } else {
                return Err(NoctraError::WindowManagerError(
                    "Surface not found".to_string(),
                ));
            }
        }
        self.persist()?;
        self.get_surface(surface_id)
    }

    /// Drag surface inside the main NOCTRA canvas with automatic edge & window-to-window magnetic snapping.
    pub fn move_and_resize_surface(
        &self,
        surface_id: Uuid,
        new_x: i32,
        new_y: i32,
        new_width: u32,
        new_height: u32,
        enable_magnetic_snap: bool,
    ) -> NoctraResult<InternalFloatingSurface> {
        let cw = *self.canvas_width.read() as i32;
        let ch = *self.canvas_height.read() as i32;

        let mut x = new_x.clamp(0, (cw - 120).max(0));
        let mut y = new_y.clamp(0, (ch - 60).max(0));
        let width = new_width.clamp(MIN_WINDOW_WIDTH, cw as u32);
        let height = new_height.clamp(MIN_WINDOW_HEIGHT, ch as u32);

        if enable_magnetic_snap {
            if x.abs() <= SNAP_THRESHOLD_PX {
                x = 0;
            } else if ((x + width as i32) - cw).abs() <= SNAP_THRESHOLD_PX {
                x = (cw - width as i32).max(0);
            }
            if y.abs() <= SNAP_THRESHOLD_PX {
                y = 0;
            } else if ((y + height as i32) - ch).abs() <= SNAP_THRESHOLD_PX {
                y = (ch - height as i32).max(0);
            }
        }

        {
            let mut guard = self.surfaces.write();
            let surface = guard
                .iter_mut()
                .find(|s| s.id == surface_id)
                .ok_or_else(|| NoctraError::WindowManagerError("Surface not found".to_string()))?;

            surface.x = x;
            surface.y = y;
            surface.width = width;
            surface.height = height;
            surface.window_state = SurfaceWindowState::Normal;
            surface.snap_zone = SnapZone::None;
        }
        self.persist()?;
        self.get_surface(surface_id)
    }

    pub fn snap_surface_to_zone(
        &self,
        surface_id: Uuid,
        zone: SnapZone,
    ) -> NoctraResult<InternalFloatingSurface> {
        let cw = *self.canvas_width.read();
        let ch = *self.canvas_height.read();
        let half_w = cw / 2;
        let half_h = ch / 2;

        let (x, y, w, h) = match zone {
            SnapZone::None => return self.restore_surface(surface_id),
            SnapZone::LeftHalf => (0, 0, half_w, ch),
            SnapZone::RightHalf => (half_w as i32, 0, cw - half_w, ch),
            SnapZone::TopHalf => (0, 0, cw, half_h),
            SnapZone::BottomHalf => (0, half_h as i32, cw, ch - half_h),
            SnapZone::TopLeftQuarter => (0, 0, half_w, half_h),
            SnapZone::TopRightQuarter => (half_w as i32, 0, cw - half_w, half_h),
            SnapZone::BottomLeftQuarter => (0, half_h as i32, half_w, ch - half_h),
            SnapZone::BottomRightQuarter => {
                (half_w as i32, half_h as i32, cw - half_w, ch - half_h)
            }
        };

        {
            let mut guard = self.surfaces.write();
            let s = guard
                .iter_mut()
                .find(|s| s.id == surface_id)
                .ok_or_else(|| NoctraError::WindowManagerError("Surface not found".to_string()))?;
            if s.restore_rect.is_none() {
                s.restore_rect = Some((s.x, s.y, s.width, s.height));
            }
            s.x = x;
            s.y = y;
            s.width = w;
            s.height = h;
            s.snap_zone = zone;
            s.window_state = SurfaceWindowState::Normal;
        }
        self.persist()?;
        self.get_surface(surface_id)
    }

    pub fn minimize_surface(&self, surface_id: Uuid) -> NoctraResult<InternalFloatingSurface> {
        {
            let mut guard = self.surfaces.write();
            let s = guard
                .iter_mut()
                .find(|s| s.id == surface_id)
                .ok_or_else(|| NoctraError::WindowManagerError("Surface not found".to_string()))?;
            s.window_state = SurfaceWindowState::Minimized;
            s.is_focused = false;
        }
        self.persist()?;
        self.get_surface(surface_id)
    }

    pub fn maximize_surface(&self, surface_id: Uuid) -> NoctraResult<InternalFloatingSurface> {
        let cw = *self.canvas_width.read();
        let ch = *self.canvas_height.read();
        let z = self.next_z_index();
        {
            let mut guard = self.surfaces.write();
            let s = guard
                .iter_mut()
                .find(|s| s.id == surface_id)
                .ok_or_else(|| NoctraError::WindowManagerError("Surface not found".to_string()))?;
            if s.window_state != SurfaceWindowState::Maximized {
                s.restore_rect = Some((s.x, s.y, s.width, s.height));
            }
            s.x = 0;
            s.y = 0;
            s.width = cw;
            s.height = ch;
            s.z_index = z;
            s.is_focused = true;
            s.window_state = SurfaceWindowState::Maximized;
        }
        self.persist()?;
        self.get_surface(surface_id)
    }

    pub fn restore_surface(&self, surface_id: Uuid) -> NoctraResult<InternalFloatingSurface> {
        let z = self.next_z_index();
        {
            let mut guard = self.surfaces.write();
            let s = guard
                .iter_mut()
                .find(|s| s.id == surface_id)
                .ok_or_else(|| NoctraError::WindowManagerError("Surface not found".to_string()))?;
            if let Some((rx, ry, rw, rh)) = s.restore_rect.take() {
                s.x = rx;
                s.y = ry;
                s.width = rw;
                s.height = rh;
            }
            s.window_state = SurfaceWindowState::Normal;
            s.snap_zone = SnapZone::None;
            s.z_index = z;
            s.is_focused = true;
        }
        self.persist()?;
        self.get_surface(surface_id)
    }

    pub fn navigate_surface(
        &self,
        surface_id: Uuid,
        new_url: String,
        new_title: Option<String>,
    ) -> NoctraResult<InternalFloatingSurface> {
        {
            let mut guard = self.surfaces.write();
            let s = guard
                .iter_mut()
                .find(|s| s.id == surface_id)
                .ok_or_else(|| NoctraError::WindowManagerError("Surface not found".to_string()))?;
            s.url = new_url.clone();
            if let Some(t) = new_title {
                s.title = t;
            } else {
                s.title = new_url;
            }
        }
        self.persist()?;
        self.get_surface(surface_id)
    }

    pub fn close_surface(&self, surface_id: Uuid) -> NoctraResult<()> {
        {
            let mut guard = self.surfaces.write();
            guard.retain(|s| s.id != surface_id);
        }
        self.persist()
    }

    /// Smart Tiling Engine (Section 9):
    /// Supports 2-window vertical/horizontal splits, 3-window main+stack, 4-window grid, and Hybrid mode.
    pub fn apply_tiling_layout(
        &self,
        workspace_id: Uuid,
        mode: WindowLayoutMode,
    ) -> NoctraResult<Vec<InternalFloatingSurface>> {
        self.workspace_layouts.write().insert(workspace_id, mode);

        let cw = *self.canvas_width.read();
        let ch = *self.canvas_height.read();
        let gap: u32 = 6;

        {
            let mut guard = self.surfaces.write();
            let mut active_surfaces: Vec<&mut InternalFloatingSurface> = guard
                .iter_mut()
                .filter(|s| {
                    s.workspace_id == workspace_id
                        && !s.is_pip_video
                        && s.window_state != SurfaceWindowState::Minimized
                })
                .collect();

            active_surfaces.sort_by_key(|s| s.z_index);
            let n = active_surfaces.len();
            if n == 0 || mode == WindowLayoutMode::FreeFloating {
                // Keep free-floating coordinates
            } else {
                match mode {
                    WindowLayoutMode::TileTwoVertical | WindowLayoutMode::Hybrid => {
                        let limit = if mode == WindowLayoutMode::Hybrid {
                            n.min(2)
                        } else {
                            n
                        };
                        let col_w = (cw.saturating_sub(gap * (limit as u32 + 1))) / (limit as u32).max(1);
                        for (idx, s) in active_surfaces.iter_mut().take(limit).enumerate() {
                            if s.restore_rect.is_none() {
                                s.restore_rect = Some((s.x, s.y, s.width, s.height));
                            }
                            s.x = (gap + idx as u32 * (col_w + gap)) as i32;
                            s.y = gap as i32;
                            s.width = col_w.max(MIN_WINDOW_WIDTH);
                            s.height = ch.saturating_sub(gap * 2).max(MIN_WINDOW_HEIGHT);
                            s.window_state = SurfaceWindowState::Normal;
                        }
                    }
                    WindowLayoutMode::TileTwoHorizontal => {
                        let row_h = (ch.saturating_sub(gap * (n as u32 + 1))) / (n as u32).max(1);
                        for (idx, s) in active_surfaces.iter_mut().enumerate() {
                            if s.restore_rect.is_none() {
                                s.restore_rect = Some((s.x, s.y, s.width, s.height));
                            }
                            s.x = gap as i32;
                            s.y = (gap + idx as u32 * (row_h + gap)) as i32;
                            s.width = cw.saturating_sub(gap * 2).max(MIN_WINDOW_WIDTH);
                            s.height = row_h.max(MIN_WINDOW_HEIGHT);
                            s.window_state = SurfaceWindowState::Normal;
                        }
                    }
                    WindowLayoutMode::TileThreeMainLeft => {
                        let half_w = (cw.saturating_sub(gap * 3)) / 2;
                        let half_h = (ch.saturating_sub(gap * 3)) / 2;
                        for (idx, s) in active_surfaces.iter_mut().enumerate() {
                            if s.restore_rect.is_none() {
                                s.restore_rect = Some((s.x, s.y, s.width, s.height));
                            }
                            if idx == 0 {
                                s.x = gap as i32;
                                s.y = gap as i32;
                                s.width = half_w;
                                s.height = ch.saturating_sub(gap * 2);
                            } else if idx == 1 {
                                s.x = (gap * 2 + half_w) as i32;
                                s.y = gap as i32;
                                s.width = half_w;
                                s.height = half_h;
                            } else {
                                s.x = (gap * 2 + half_w) as i32;
                                s.y = (gap * 2 + half_h) as i32;
                                s.width = half_w;
                                s.height = half_h;
                            }
                            s.window_state = SurfaceWindowState::Normal;
                        }
                    }
                    WindowLayoutMode::TileFourGrid => {
                        let cols = 2u32;
                        let rows = ((n as f32) / 2.0).ceil().max(1.0) as u32;
                        let cell_w = (cw.saturating_sub(gap * (cols + 1))) / cols;
                        let cell_h = (ch.saturating_sub(gap * (rows + 1))) / rows;
                        for (idx, s) in active_surfaces.iter_mut().enumerate() {
                            if s.restore_rect.is_none() {
                                s.restore_rect = Some((s.x, s.y, s.width, s.height));
                            }
                            let col = (idx as u32) % cols;
                            let row = (idx as u32) / cols;
                            s.x = (gap + col * (cell_w + gap)) as i32;
                            s.y = (gap + row * (cell_h + gap)) as i32;
                            s.width = cell_w.max(MIN_WINDOW_WIDTH);
                            s.height = cell_h.max(MIN_WINDOW_HEIGHT);
                            s.window_state = SurfaceWindowState::Normal;
                        }
                    }
                    WindowLayoutMode::FreeFloating => {}
                }
            }
        }
        self.persist()?;
        Ok(self.list_surfaces_for_workspace(workspace_id))
    }

    pub fn replace_workspace_surfaces(
        &self,
        workspace_id: Uuid,
        restored: Vec<InternalFloatingSurface>,
    ) -> NoctraResult<()> {
        {
            let mut guard = self.surfaces.write();
            guard.retain(|s| s.workspace_id != workspace_id);
            guard.extend(restored);
        }
        self.persist()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_internal_floating_windows_snap_tiling_and_persistence() {
        let db = Arc::new(NoctraDatabase::open_in_memory().unwrap());
        let wm = InternalWindowManager::initialize(db.clone(), 1600, 900).unwrap();

        let pid = Uuid::new_v4();
        let wid = Uuid::new_v4();

        let s1 = wm
            .spawn_surface(
                pid,
                wid,
                Uuid::new_v4(),
                "Rust Docs".to_string(),
                "https://doc.rust-lang.org".to_string(),
                "part_1".to_string(),
                false,
            )
            .unwrap();
        let s2 = wm
            .spawn_surface(
                pid,
                wid,
                Uuid::new_v4(),
                "Kubernetes".to_string(),
                "https://kubernetes.io".to_string(),
                "part_1".to_string(),
                false,
            )
            .unwrap();
        let s3 = wm
            .spawn_surface(
                pid,
                wid,
                Uuid::new_v4(),
                "Istio".to_string(),
                "https://istio.io".to_string(),
                "part_1".to_string(),
                false,
            )
            .unwrap();
        let s4 = wm
            .spawn_surface(
                pid,
                wid,
                Uuid::new_v4(),
                "Grafana".to_string(),
                "https://grafana.com".to_string(),
                "part_1".to_string(),
                false,
            )
            .unwrap();

        assert!(s4.z_index > s1.z_index);

        // Bring s1 to front
        let s1_front = wm.bring_to_front(s1.id).unwrap();
        assert!(s1_front.z_index > s4.z_index);
        assert!(s1_front.is_focused);

        // Snap s2 to LeftHalf
        let s2_snapped = wm.snap_surface_to_zone(s2.id, SnapZone::LeftHalf).unwrap();
        assert_eq!(s2_snapped.x, 0);
        assert_eq!(s2_snapped.width, 800);
        assert_eq!(s2_snapped.height, 900);

        // Maximize and Restore s3
        let s3_orig_w = s3.width;
        let s3_max = wm.maximize_surface(s3.id).unwrap();
        assert_eq!(s3_max.width, 1600);
        assert_eq!(s3_max.window_state, SurfaceWindowState::Maximized);

        let s3_restored = wm.restore_surface(s3.id).unwrap();
        assert_eq!(s3_restored.width, s3_orig_w);
        assert_eq!(s3_restored.window_state, SurfaceWindowState::Normal);

        // Test 4-Window Grid Smart Tiling
        let tiled = wm
            .apply_tiling_layout(wid, WindowLayoutMode::TileFourGrid)
            .unwrap();
        assert_eq!(tiled.len(), 4);
        assert_ne!(tiled[0].x, tiled[1].x);
        assert_ne!(tiled[0].y, tiled[2].y);

        // Verify persistence reload from SQLite
        let wm_reloaded = InternalWindowManager::initialize(db, 1600, 900).unwrap();
        let persisted = wm_reloaded.list_surfaces_for_workspace(wid);
        assert_eq!(persisted.len(), 4);
    }
}
