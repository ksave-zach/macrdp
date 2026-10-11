use ironrdp_server::{KeyboardEvent, MouseEvent, RdpServerInputHandler};
use macrdp_input::{KeyboardInjector, MouseButton, MouseInjector};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Maps RDP desktop coordinates to macOS logical coordinates.
///
/// Completely independent of capture/encode resolution — only cares about
/// the RDP desktop size (what the client thinks) and the macOS logical
/// display size (what CGEvent expects).
///
/// Formula: `mac = rdp × logical ÷ rdp_desktop`
///
/// The macOS logical size is re-read at most once per second on each
/// mapping, so later display configuration changes are picked up without
/// restarting the server.
#[derive(Debug, Clone)]
pub struct MouseCoordMapper(Arc<Mutex<CoordState>>);

#[derive(Debug)]
struct CoordState {
    logical_w: f64,
    logical_h: f64,
    rdp_w: f64,
    rdp_h: f64,
    last_check: Instant,
}

impl MouseCoordMapper {
    pub fn new(logical_w: u16, logical_h: u16, rdp_w: u16, rdp_h: u16) -> Self {
        tracing::info!(
            logical_w, logical_h, rdp_w, rdp_h,
            "MouseCoordMapper initialized"
        );
        Self(Arc::new(Mutex::new(CoordState {
            logical_w: logical_w as f64,
            logical_h: logical_h as f64,
            rdp_w: rdp_w.max(1) as f64,
            rdp_h: rdp_h.max(1) as f64,
            last_check: Instant::now(),
        })))
    }

    /// Re-read the live CG display size (throttled to once per second) and
    /// update the mapping basis if the display configuration changed.
    /// Without this, the basis stays at the value captured at startup and
    /// the cursor drifts off by the ratio between old and new sizes.
    fn refresh_logical_size(&self) {
        {
            let mut s = self.0.lock().unwrap();
            if s.last_check.elapsed() < Duration::from_secs(1) {
                return;
            }
            s.last_check = Instant::now();
        }

        let Ok((w, h)) = macrdp_capture::detect_cg_display_size() else {
            return;
        };

        let mut s = self.0.lock().unwrap();
        if w as f64 != s.logical_w || h as f64 != s.logical_h {
            tracing::info!(
                old_w = s.logical_w, old_h = s.logical_h,
                new_w = w, new_h = h,
                "MouseCoordMapper: display size changed — mapping basis updated"
            );
            s.logical_w = w as f64;
            s.logical_h = h as f64;
        }
    }

    pub fn map(&self, rdp_x: u16, rdp_y: u16) -> (u16, u16) {
        self.refresh_logical_size();
        let s = self.0.lock().unwrap();
        let mac_x = (rdp_x as f64 * s.logical_w / s.rdp_w) as u16;
        let mac_y = (rdp_y as f64 * s.logical_h / s.rdp_h) as u16;
        (mac_x, mac_y)
    }

    pub fn update_rdp_size(&self, rdp_w: u16, rdp_h: u16) {
        let mut s = self.0.lock().unwrap();
        let old_w = s.rdp_w;
        let old_h = s.rdp_h;
        s.rdp_w = rdp_w.max(1) as f64;
        s.rdp_h = rdp_h.max(1) as f64;
        tracing::info!(
            old_rdp_w = old_w, old_rdp_h = old_h,
            new_rdp_w = rdp_w, new_rdp_h = rdp_h,
            logical_w = s.logical_w, logical_h = s.logical_h,
            "MouseCoordMapper: RDP desktop size updated"
        );
    }
}

/// Bridges RDP input events to macOS CGEvent injection
pub struct MacInputHandler {
    keyboard: Option<KeyboardInjector>,
    mouse: Option<MouseInjector>,
    last_mouse_x: u16,
    last_mouse_y: u16,
    coord_mapper: MouseCoordMapper,
}

impl MacInputHandler {
    pub fn new(coord_mapper: MouseCoordMapper) -> Self {
        let keyboard = KeyboardInjector::new()
            .map_err(|e| tracing::error!("Failed to create keyboard injector: {e}"))
            .ok();
        let mouse = MouseInjector::new()
            .map_err(|e| tracing::error!("Failed to create mouse injector: {e}"))
            .ok();

        if keyboard.is_none() || mouse.is_none() {
            tracing::warn!(
                "Input injection may fail — ensure Accessibility permission is granted"
            );
        }

        Self {
            keyboard,
            mouse,
            last_mouse_x: 0,
            last_mouse_y: 0,
            coord_mapper,
        }
    }

}

impl RdpServerInputHandler for MacInputHandler {
    fn on_connect(&mut self) {
        // Briefly prevent display sleep to wake the loginwindow session
        // so the "ABC" keyboard input source is reloaded. Without this,
        // synthetic key events produce wrong characters in the login
        // screen password field when the display has been idle.
        let _ = std::process::Command::new("caffeinate")
            .args(["-d", "-u", "-t", "3"])
            .spawn();
        tracing::info!("Client connected — caffeinate poke sent");
    }

    fn keyboard(&mut self, event: KeyboardEvent) {
        let Some(kb) = &self.keyboard else { return };

        let result = match event {
            KeyboardEvent::Pressed { code, extended } => kb.inject_key(code, extended, true),
            KeyboardEvent::Released { code, extended } => kb.inject_key(code, extended, false),
            KeyboardEvent::UnicodePressed(ch) => kb.inject_unicode(ch, true),
            KeyboardEvent::UnicodeReleased(ch) => kb.inject_unicode(ch, false),
            KeyboardEvent::Synchronize(_flags) => {
                tracing::debug!("Keyboard synchronize event (ignored)");
                Ok(())
            }
        };

        if let Err(e) = result {
            tracing::warn!("Keyboard injection failed: {e}");
        }
    }

    fn mouse(&mut self, event: MouseEvent) {
        let Some(m) = &self.mouse else { return };

        // Map coordinates first (before borrowing self.mouse for injection)
        let (mx, my) = match &event {
            MouseEvent::Move { x, y }
            | MouseEvent::LeftPressed { x, y }
            | MouseEvent::LeftReleased { x, y }
            | MouseEvent::RightPressed { x, y }
            | MouseEvent::RightReleased { x, y }
            | MouseEvent::MiddlePressed { x, y }
            | MouseEvent::MiddleReleased { x, y } => {
                let result = self.coord_mapper.map(*x, *y);
                self.last_mouse_x = result.0;
                self.last_mouse_y = result.1;
                result
            }
            _ => (self.last_mouse_x, self.last_mouse_y),
        };

        let result = match event {
            MouseEvent::Move { .. } => m.move_to(mx, my),
            MouseEvent::LeftPressed { .. } => m.button_event(MouseButton::Left, true, mx, my),
            MouseEvent::LeftReleased { .. } => m.button_event(MouseButton::Left, false, mx, my),
            MouseEvent::RightPressed { .. } => m.button_event(MouseButton::Right, true, mx, my),
            MouseEvent::RightReleased { .. } => m.button_event(MouseButton::Right, false, mx, my),
            MouseEvent::MiddlePressed { .. } => m.button_event(MouseButton::Middle, true, mx, my),
            MouseEvent::MiddleReleased { .. } => m.button_event(MouseButton::Middle, false, mx, my),
            MouseEvent::VerticalScroll { value } => m.scroll(value),
            _ => {
                tracing::trace!(?event, "Unhandled mouse event");
                Ok(())
            }
        };

        if let Err(e) = result {
            tracing::warn!("Mouse injection failed: {e}");
        }
    }
}
