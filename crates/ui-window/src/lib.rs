//! Native window lifecycle and HiDPI metrics. No renderer dependency.

use ui_core::{ScaleFactor, Size};
use winit::{
    dpi::{LogicalSize, PhysicalSize},
    event_loop::ActiveEventLoop,
    window::{Window, WindowAttributes, WindowId},
};

/// IME events normalized for runtime consumers. Preedit cursor offsets are grapheme indices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlatformImeEvent {
    Enabled,
    Preedit {
        text: String,
        cursor: Option<(usize, usize)>,
    },
    Commit(String),
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlatformTextCommand {
    SelectAll,
    Copy,
    Cut,
    Paste,
    Undo,
    Redo,
    DeleteBackward {
        word: bool,
    },
    DeleteForward {
        word: bool,
    },
    MoveLeft {
        word: bool,
        line: bool,
        extend: bool,
    },
    MoveRight {
        word: bool,
        line: bool,
        extend: bool,
    },
    MoveUp {
        extend: bool,
    },
    MoveDown {
        extend: bool,
    },
    LineStart {
        document: bool,
        extend: bool,
    },
    LineEnd {
        document: bool,
        extend: bool,
    },
    InsertLineBreak,
    CancelComposition,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformTextInput {
    pub command: Option<PlatformTextCommand>,
    pub text: Option<String>,
    pub pressed: bool,
    pub repeat: bool,
}

impl PlatformTextInput {
    /// Map winit key/modifier conventions at the window boundary; editor commands stay platform-neutral.
    pub fn from_winit(
        event: &winit::event::KeyEvent,
        modifiers: winit::keyboard::ModifiersState,
    ) -> Self {
        let command = map_text_command(&event.logical_key, modifiers);
        let is_altgr = modifiers.control_key() && modifiers.alt_key() && !modifiers.super_key();
        let text = if modifiers.super_key() || (modifiers.control_key() && !is_altgr) {
            None
        } else {
            event
                .text
                .as_ref()
                .map(ToString::to_string)
                .filter(|value| !value.is_empty())
        };
        Self {
            command,
            text,
            pressed: event.state == winit::event::ElementState::Pressed,
            repeat: event.repeat,
        }
    }
}

fn map_text_command(
    key: &winit::keyboard::Key,
    modifiers: winit::keyboard::ModifiersState,
) -> Option<PlatformTextCommand> {
    use winit::keyboard::{Key, NamedKey};
    let shift = modifiers.shift_key();
    let is_macos = cfg!(target_os = "macos");
    let control_or_command =
        modifiers.super_key() || (modifiers.control_key() && !modifiers.alt_key());
    let word = if is_macos {
        modifiers.alt_key()
    } else {
        modifiers.control_key() && !modifiers.alt_key()
    };
    let line = is_macos && modifiers.super_key();
    let shortcut = if control_or_command {
        if let Key::Character(key) = key {
            match key.to_lowercase().as_str() {
                "a" => Some(PlatformTextCommand::SelectAll),
                "c" => Some(PlatformTextCommand::Copy),
                "x" => Some(PlatformTextCommand::Cut),
                "v" => Some(PlatformTextCommand::Paste),
                "z" if shift => Some(PlatformTextCommand::Redo),
                "z" => Some(PlatformTextCommand::Undo),
                "y" => Some(PlatformTextCommand::Redo),
                _ => None,
            }
        } else {
            None
        }
    } else {
        None
    };
    shortcut.or(match key {
        Key::Named(NamedKey::Backspace) => Some(PlatformTextCommand::DeleteBackward { word }),
        Key::Named(NamedKey::Delete) => Some(PlatformTextCommand::DeleteForward { word }),
        Key::Named(NamedKey::ArrowLeft) => Some(PlatformTextCommand::MoveLeft {
            word,
            line,
            extend: shift,
        }),
        Key::Named(NamedKey::ArrowRight) => Some(PlatformTextCommand::MoveRight {
            word,
            line,
            extend: shift,
        }),
        Key::Named(NamedKey::ArrowUp) => Some(PlatformTextCommand::MoveUp { extend: shift }),
        Key::Named(NamedKey::ArrowDown) => Some(PlatformTextCommand::MoveDown { extend: shift }),
        Key::Named(NamedKey::Home) => Some(PlatformTextCommand::LineStart {
            document: control_or_command,
            extend: shift,
        }),
        Key::Named(NamedKey::End) => Some(PlatformTextCommand::LineEnd {
            document: control_or_command,
            extend: shift,
        }),
        Key::Named(NamedKey::Enter) => Some(PlatformTextCommand::InsertLineBreak),
        Key::Named(NamedKey::Escape) => Some(PlatformTextCommand::CancelComposition),
        _ => None,
    })
}

impl From<winit::event::Ime> for PlatformImeEvent {
    fn from(event: winit::event::Ime) -> Self {
        use unicode_segmentation::UnicodeSegmentation;
        match event {
            winit::event::Ime::Enabled => Self::Enabled,
            winit::event::Ime::Preedit(text, cursor) => {
                let cursor = cursor.map(|(start, end)| {
                    let start = start.min(text.len());
                    let end = end.min(text.len());
                    let start = (0..=start)
                        .rev()
                        .find(|i| text.is_char_boundary(*i))
                        .unwrap_or(0);
                    let end = (0..=end)
                        .rev()
                        .find(|i| text.is_char_boundary(*i))
                        .unwrap_or(0);
                    (
                        text[..start].graphemes(true).count(),
                        text[..end].graphemes(true).count(),
                    )
                });
                Self::Preedit { text, cursor }
            }
            winit::event::Ime::Commit(text) => Self::Commit(text),
            winit::event::Ime::Disabled => Self::Disabled,
        }
    }
}

#[derive(Clone, Debug)]
pub struct WindowConfig {
    pub title: String,
    pub logical_size: Size,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "UI Runtime Foundation".to_owned(),
            logical_size: Size::new(1120.0, 720.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowMetrics {
    pub logical_size: Size,
    pub physical_size: [u32; 2],
    pub scale_factor: ScaleFactor,
}

impl WindowMetrics {
    pub fn from_window(window: &Window) -> Self {
        let scale_factor = ScaleFactor::new(window.scale_factor() as f32);
        let physical_size = window.inner_size();
        let logical_size = Size::new(
            physical_size.width as f32 / scale_factor.get(),
            physical_size.height as f32 / scale_factor.get(),
        );
        Self {
            logical_size,
            physical_size: [physical_size.width, physical_size.height],
            scale_factor,
        }
    }
}

pub struct UiWindow {
    window: std::sync::Arc<Window>,
    metrics: WindowMetrics,
}

impl UiWindow {
    pub fn open(
        event_loop: &ActiveEventLoop,
        config: &WindowConfig,
    ) -> Result<Self, winit::error::OsError> {
        let attributes = WindowAttributes::default()
            .with_title(config.title.clone())
            .with_inner_size(LogicalSize::new(
                config.logical_size.width,
                config.logical_size.height,
            ));
        let window = std::sync::Arc::new(event_loop.create_window(attributes)?);
        let metrics = WindowMetrics::from_window(&window);
        Ok(Self { window, metrics })
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }
    pub fn window(&self) -> &std::sync::Arc<Window> {
        &self.window
    }
    pub fn metrics(&self) -> WindowMetrics {
        self.metrics
    }
    pub fn refresh_metrics(&mut self) {
        self.metrics = WindowMetrics::from_window(&self.window);
    }
    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }
    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width > 0 && size.height > 0 {
            self.metrics.physical_size = [size.width, size.height];
            self.metrics.logical_size = Size::new(
                size.width as f32 / self.metrics.scale_factor.get(),
                size.height as f32 / self.metrics.scale_factor.get(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ime_preedit_byte_offsets_become_grapheme_offsets() {
        let event = PlatformImeEvent::from(winit::event::Ime::Preedit(
            "e\u{301}👨‍👩‍👧‍👦".into(),
            Some((3, 3)),
        ));
        assert_eq!(
            event,
            PlatformImeEvent::Preedit {
                text: "e\u{301}👨‍👩‍👧‍👦".into(),
                cursor: Some((1, 1)),
            }
        );
    }
    #[test]
    fn normalized_key_mapping_keeps_shift_selection_and_altgr_text_unbound() {
        use winit::keyboard::{Key, ModifiersState, NamedKey};
        assert_eq!(
            map_text_command(&Key::Named(NamedKey::ArrowLeft), ModifiersState::SHIFT),
            Some(PlatformTextCommand::MoveLeft {
                word: false,
                line: false,
                extend: true
            }),
        );
        assert_eq!(
            map_text_command(
                &Key::Character("@".into()),
                ModifiersState::CONTROL | ModifiersState::ALT
            ),
            None,
        );
        assert_eq!(
            map_text_command(&Key::Character("a".into()), ModifiersState::SUPER),
            Some(PlatformTextCommand::SelectAll),
        );
    }
}
