use anyhow::Result;
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, CGKeyCode};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use std::sync::Mutex;

use crate::keymap::scancode_to_keycode;

pub struct KeyboardInjector {
    /// Currently held modifier flags (tracked manually because synthetic
    /// CGEvent posts do not update the system's internal modifier state).
    modifiers: Mutex<CGEventFlags>,
}

/// Map a macOS keycode to the CGEventFlags it controls (if it's a modifier).
fn modifier_flag_for_keycode(keycode: u16) -> Option<CGEventFlags> {
    match keycode {
        0x38 | 0x3C => Some(CGEventFlags::CGEventFlagShift),     // Left/Right Shift
        0x3B | 0x3E => Some(CGEventFlags::CGEventFlagControl),   // Left/Right Control
        0x3A | 0x3D => Some(CGEventFlags::CGEventFlagAlternate), // Left/Right Option
        0x37 | 0x36 => Some(CGEventFlags::CGEventFlagCommand),   // Left/Right Command
        0x39 => Some(CGEventFlags::CGEventFlagAlphaShift),        // Caps Lock
        _ => None,
    }
}

impl KeyboardInjector {
    pub fn new() -> Result<Self> {
        // Verify we can create an event source (permission check)
        let _ = CGEventSource::new(CGEventSourceStateID::Private)
            .map_err(|_| anyhow::anyhow!(
                "Failed to create CGEventSource — check Accessibility permission"
            ))?;
        Ok(Self {
            modifiers: Mutex::new(CGEventFlags::empty()),
        })
    }

    fn source() -> Result<CGEventSource> {
        CGEventSource::new(CGEventSourceStateID::Private)
            .map_err(|_| anyhow::anyhow!("Failed to create CGEventSource"))
    }

    /// Update tracked modifier state and return the flags to apply to any
    /// subsequent keyboard event.
    fn apply_modifier(&self, keycode: u16, pressed: bool) -> CGEventFlags {
        let mut flags = self.modifiers.lock().unwrap();
        if let Some(flag) = modifier_flag_for_keycode(keycode) {
            if pressed {
                // Caps Lock toggles on press only; never clear on release
                if keycode == 0x39 {
                    flags.toggle(flag);
                } else {
                    flags.insert(flag);
                }
            } else if keycode != 0x39 {
                flags.remove(flag);
            }
        }
        *flags
    }

    /// Inject a key press or release event with proper modifier flag tracking.
    pub fn inject_key(&self, scancode: u8, extended: bool, pressed: bool) -> Result<()> {
        let keycode = match scancode_to_keycode(scancode, extended) {
            Some(kc) => kc,
            None => {
                tracing::warn!(scancode, extended, "Unknown scancode, ignoring");
                return Ok(());
            }
        };

        let source = Self::source()?;
        let event = CGEvent::new_keyboard_event(source, keycode as CGKeyCode, pressed)
            .map_err(|_| anyhow::anyhow!("Failed to create keyboard event"))?;

        // Manually set modifier flags because synthetic events do not update
        // the system's internal modifier state (Shift/Ctrl/Option/Cmd held).
        let flags = self.apply_modifier(keycode, pressed);
        if !flags.is_empty() {
            event.set_flags(flags);
        }

        event.post(CGEventTapLocation::HID);
        tracing::trace!(scancode, keycode, pressed, ?flags, "Keyboard event injected");
        Ok(())
    }

    /// Inject a unicode character press/release.
    pub fn inject_unicode(&self, ch: u16, pressed: bool) -> Result<()> {
        let source = Self::source()?;
        let event = CGEvent::new_keyboard_event(source, 0, pressed)
            .map_err(|_| anyhow::anyhow!("Failed to create unicode event"))?;

        if pressed {
            event.set_string_from_utf16_unchecked(&[ch]);
        }

        // Apply currently held modifier flags (no keycode change for unicode)
        let flags = *self.modifiers.lock().unwrap();
        if !flags.is_empty() {
            event.set_flags(flags);
        }

        event.post(CGEventTapLocation::HID);
        Ok(())
    }
}
