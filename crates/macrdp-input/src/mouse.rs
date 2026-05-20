use anyhow::Result;
use core_graphics::event::{CGEvent, CGEventTapLocation, CGEventType, CGMouseButton};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;
use foreign_types::ForeignType;
use std::sync::Mutex;

extern "C" {
    fn CGEventCreateScrollWheelEvent2(
        source: *mut core_graphics::sys::CGEventSource,
        units: u32,
        wheel_count: u32,
        wheel1: i32,
        wheel2: i32,
        wheel3: i32,
    ) -> *mut core_graphics::sys::CGEvent;
}

/// ScrollEventUnit::Line
const SCROLL_UNIT_LINE: u32 = 1;

pub struct MouseInjector {
    /// Track which button is currently pressed so move events can use the
    /// correct *Dragged event type (LeftMouseDragged etc.) instead of
    /// MouseMoved — otherwise drag operations won't work.
    held_button: Mutex<Option<(CGMouseButton, MouseButton)>>,
}

impl MouseInjector {
    pub fn new() -> Result<Self> {
        let _ = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .map_err(|_| anyhow::anyhow!("Failed to create CGEventSource — check Accessibility permission"))?;
        Ok(Self {
            held_button: Mutex::new(None),
        })
    }

    fn source() -> Result<CGEventSource> {
        CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .map_err(|_| anyhow::anyhow!("Failed to create CGEventSource"))
    }

    pub fn move_to(&self, x: u16, y: u16) -> Result<()> {
        let point = CGPoint::new(x as f64, y as f64);
        let source = Self::source()?;

        let (event_type, cg_button) = match *self.held_button.lock().unwrap() {
            Some((cg_btn, _)) => match cg_btn {
                CGMouseButton::Left => (CGEventType::LeftMouseDragged, CGMouseButton::Left),
                CGMouseButton::Right => (CGEventType::RightMouseDragged, CGMouseButton::Right),
                CGMouseButton::Center => (CGEventType::OtherMouseDragged, CGMouseButton::Center),
            },
            None => (CGEventType::MouseMoved, CGMouseButton::Left),
        };

        let event = CGEvent::new_mouse_event(source, event_type, point, cg_button)
            .map_err(|_| anyhow::anyhow!("Failed to create mouse move event"))?;

        event.post(CGEventTapLocation::HID);
        tracing::trace!(x, y, ?event_type, "Mouse moved");
        Ok(())
    }

    pub fn button_event(&self, button: MouseButton, pressed: bool, x: u16, y: u16) -> Result<()> {
        let point = CGPoint::new(x as f64, y as f64);
        let (event_type, cg_button) = match (button, pressed) {
            (MouseButton::Left, true) => (CGEventType::LeftMouseDown, CGMouseButton::Left),
            (MouseButton::Left, false) => (CGEventType::LeftMouseUp, CGMouseButton::Left),
            (MouseButton::Right, true) => (CGEventType::RightMouseDown, CGMouseButton::Right),
            (MouseButton::Right, false) => (CGEventType::RightMouseUp, CGMouseButton::Right),
            (MouseButton::Middle, true) => (CGEventType::OtherMouseDown, CGMouseButton::Center),
            (MouseButton::Middle, false) => (CGEventType::OtherMouseUp, CGMouseButton::Center),
        };

        // Track button state so move_to can emit the correct Dragged event
        {
            let mut held = self.held_button.lock().unwrap();
            if pressed {
                *held = Some((cg_button, button));
            } else {
                *held = None;
            }
        }

        let source = Self::source()?;
        let event = CGEvent::new_mouse_event(source, event_type, point, cg_button)
            .map_err(|_| anyhow::anyhow!("Failed to create mouse button event"))?;

        event.post(CGEventTapLocation::HID);
        tracing::trace!(?button, pressed, x, y, "Mouse button event");
        Ok(())
    }

    pub fn scroll(&self, vertical: i16) -> Result<()> {
        // RDP sends wheel rotation units (±120 per notch), but CGEvent
        // expects line count. Scale down to ~4 lines per notch.
        let lines = vertical as i32 / 30;
        unsafe {
            let event_ref = CGEventCreateScrollWheelEvent2(
                std::ptr::null_mut(),
                SCROLL_UNIT_LINE,
                1,
                lines,
                0,
                0,
            );
            if event_ref.is_null() {
                return Err(anyhow::anyhow!("Failed to create scroll event"));
            }
            let event = CGEvent::from_ptr(event_ref);
            event.post(CGEventTapLocation::HID);
        }
        tracing::trace!(vertical, "Mouse scroll");
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}
