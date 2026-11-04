//! Typing into apps (our compositor's `text_input` feature): an app with a focused text field tells
//! the compositor (`text-input-v3`), the compositor tells the shell as its input method, and ✕
//! then opens the floating keyboard in the overlay window above the app. The app keeps the
//! keyboard focus; the shell reads the controller exclusively (like in the quick menu) and types
//! with `commit_string`. R3 moves the keyboard, ○ closes it.

use std::cell::Cell;

use consoleui_core::osk::OnScreenKeyboard;

use super::*;
use crate::inputmethod::{Event as FieldEvent, InputMethod};

/// The keyboard over an app
pub(super) struct Typing {
    keyboard: OnScreenKeyboard,
    /// What was typed since the keyboard opened, for fields that do not tell their content
    typed: String,
}

impl Shell {
    /// Starts the input method once the compositor offers it, and handles what it reports.
    pub fn poll_input_method(&mut self, ui: &AppWindow) {
        if self.input_method.is_none()
            && !self.input_method_tried
            && self.compositor.as_ref().is_some_and(Compositor::text_input)
        {
            self.input_method_tried = true;
            self.input_method = InputMethod::start();
        }
        let Some(input_method) = &self.input_method else { return };
        for event in input_method.poll() {
            match event {
                FieldEvent::Active(field) => self.text_field = Some(field),
                FieldEvent::Inactive => {
                    self.text_field = None;
                    self.close_typing(ui);
                }
            }
        }
        if self.typing.is_some() {
            self.sync_typing();
        }
    }

    /// A text field of an app has the focus while that app is in front: ✕ opens the keyboard.
    pub(super) fn wants_typing_keyboard(&self, ui: &AppWindow) -> bool {
        self.typing.is_none() && self.text_field.is_some() && !self.shell_in_front(ui) && self.compositor_ui().is_some()
    }

    pub(super) fn open_typing(&mut self, ui: &AppWindow) {
        let Some(overlay) = self.compositor_ui().map(|c| c.overlay.clone_strong()) else { return };
        // With the shift layer: digits, symbols and both cases (Story: a numeric layer for fields
        // that only take digits)
        let mut keyboard = OnScreenKeyboard::password();
        keyboard.reset_focus();
        self.typing = Some(Typing { keyboard, typed: String::new() });
        self.keyboard_place.reset_mode();
        self.typing_generation.set(self.typing_generation.get() + 1);
        overlay.set_controller_style(ui.get_controller_style());
        overlay.window().set_fullscreen(true);
        if let Err(err) = overlay.show() {
            eprintln!("Cannot show the keyboard: {err}");
        }
        overlay.window().request_redraw();
        self.sync_typing();
        let (weak, generation, opened) =
            (overlay.as_weak(), self.typing_generation.clone(), self.typing_generation.get());
        slint::Timer::single_shot(Duration::from_millis(30), move || {
            if let Some(overlay) = weak.upgrade()
                && generation.get() == opened
            {
                overlay.set_keyboard_open(true);
            }
        });
    }

    pub(super) fn close_typing(&mut self, _ui: &AppWindow) {
        if self.typing.take().is_none() {
            return;
        }
        self.keyboard_place.reset_mode();
        self.typing_generation.set(self.typing_generation.get() + 1);
        let Some(overlay) = self.compositor_ui().map(|c| c.overlay.clone_strong()) else { return };
        overlay.set_keyboard_open(false);
        overlay.set_keyboard_moving(false);
        let (weak, generation, closed) =
            (overlay.as_weak(), self.typing_generation.clone(), self.typing_generation.get());
        slint::Timer::single_shot(Duration::from_millis(450), move || {
            if let Some(overlay) = weak.upgrade().filter(|o| !o.get_open() && !o.get_keyboard_open())
                && generation.get() == closed
            {
                let _ = overlay.hide();
            }
        });
    }

    /// Input while the keyboard is open: ○ closes it, R3 grabs it to move it, the rest types.
    pub(super) fn apply_typing(&mut self, ui: &AppWindow, action: Action) {
        if self.keyboard_place.apply(action) {
            self.sync_typing();
            return;
        }
        let Some(typing) = &mut self.typing else { return };
        let event = match action {
            Action::Back => {
                self.close_typing(ui);
                return;
            }
            // □ deletes, △ types a space, as on the other keyboards
            Action::Secondary => KeyboardEvent::Backspace,
            Action::Search => KeyboardEvent::Space,
            other => typing.keyboard.apply(other),
        };
        let before = self.text_field.as_ref().map(|f| f.before_cursor.clone()).unwrap_or_default();
        let Some(input_method) = &self.input_method else { return };
        match event {
            KeyboardEvent::Type(c) => {
                typing.typed.push(c);
                input_method.commit(c.to_string());
            }
            KeyboardEvent::Space => {
                typing.typed.push(' ');
                input_method.commit(" ");
            }
            KeyboardEvent::Backspace => {
                // Whole characters: the length of the last one in bytes, from what the app tells
                // or what was typed
                let last = before.chars().next_back().or_else(|| typing.typed.chars().next_back());
                typing.typed.pop();
                if let Some(last) = last {
                    input_method.delete_before(last.len_utf8() as u32);
                }
            }
            KeyboardEvent::Done => {
                self.close_typing(ui);
                return;
            }
            KeyboardEvent::Shift | KeyboardEvent::Moved | KeyboardEvent::None => {}
        }
        self.sync_typing();
    }

    /// Pushes the keyboard state to the overlay window.
    pub(super) fn sync_typing(&self) {
        let (Some(typing), Some(overlay)) = (&self.typing, self.compositor_ui().map(|c| &c.overlay)) else { return };
        let keys: Vec<KeyCap> = typing
            .keyboard
            .keys()
            .iter()
            .enumerate()
            .map(|(i, k)| KeyCap { label: typing.keyboard.label(i).into(), row: k.row as i32, x: k.x, width: k.width })
            .collect();
        overlay.set_keys(ModelRc::new(VecModel::from(keys)));
        overlay.set_key_focused(typing.keyboard.focused() as i32);
        let field = self.text_field.clone().unwrap_or_default();
        let text = if field.secret {
            "•".repeat(typing.typed.chars().count())
        } else if !field.before_cursor.is_empty() {
            let chars: Vec<char> = field.before_cursor.chars().collect();
            chars[chars.len().saturating_sub(48)..].iter().collect()
        } else {
            typing.typed.clone()
        };
        overlay.set_keyboard_text(text.into());
        overlay.set_keyboard_title(tr("Type into the app").into());
        overlay.set_keyboard_placeholder(tr("Text field").into());
        let place = self.keyboard_place.placement();
        overlay.set_keyboard_x(place.x);
        overlay.set_keyboard_y(place.y);
        overlay.set_keyboard_moving(self.keyboard_place.moving());
        let hint = |button, label: &str| Hint { button, label: label.into() };
        let hints = vec![
            hint(GlyphButton::Confirm, tr("Type")),
            hint(GlyphButton::West, tr("Delete")),
            hint(GlyphButton::North, tr("Space")),
            hint(GlyphButton::Back, tr("Close")),
        ];
        overlay.set_keyboard_hints(ModelRc::new(VecModel::from(hints)));
    }
}

/// The generation counter that keeps a late "show" or "hide" timer from acting on a newer state
pub(super) fn new_generation() -> Rc<Cell<u64>> {
    Rc::new(Cell::new(0))
}
