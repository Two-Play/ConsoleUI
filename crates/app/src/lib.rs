//! Runtime for ConsoleUI apps. The design system (`consoleui-ui`) draws the screens; this crate
//! carries what every app needs besides that:
//!
//! - [`input::Input`]: controller presses as the shell's `Action`s, gated on the window focus,
//!   plus the glyph style for the hints (keyboard keys, PlayStation or Xbox buttons)
//! - [`hints::Hints`]: the hint bar content, independent of Slint
//! - [`session::Session`]: where the app runs (desktop, our compositor, gamescope) and the user's
//!   settings (accent color, animations)
//! - [`audio::Audio`]: the UI sounds of the shell
//! - [`slint_bridge!`]: converters between these types and the ones Slint generates for the app
//!
//! The compositor needs nothing from an app: the shell registers the launched process, the
//! compositor maps its windows to it, shows them fullscreen and takes care of the Guide key.
//! Closing the app (e.g. `Action::Back` on its first screen) returns to the shell.
//!
//! A minimal app is `crates/gallery`.

pub mod audio;
pub mod gamepad;
pub mod hints;
pub mod input;
pub mod session;

pub use consoleui_core::Action;
pub use consoleui_core::input::GlyphStyle;
pub use consoleui_core::sound::Sound;
pub use hints::{HintButton, Hints};
pub use input::{Event, Input};
pub use session::Session;

/// Defines the conversions between this crate's types and the types Slint generated for the app.
/// Call it once in the module that has `slint::include_modules!()`; the UI must use `NavAction`,
/// `GlyphButton`, `ControllerStyle`, `Hint` from `@consoleui/types.slint` and export `Theme` from
/// the main component's file:
///
/// ```ignore
/// slint::include_modules!();
/// consoleui_app::slint_bridge!();
/// // then: nav_action(NavAction::Left), hint_model(hints), controller_style(glyphs), apply_theme(ui.global::<Theme>(), &session)
/// ```
#[macro_export]
macro_rules! slint_bridge {
    () => {
        /// The shell's `Action` for a key press reported by `InputScope`.
        #[allow(dead_code)]
        fn nav_action(action: NavAction) -> $crate::Action {
            match action {
                NavAction::Up => $crate::Action::Up,
                NavAction::Down => $crate::Action::Down,
                NavAction::Left => $crate::Action::Left,
                NavAction::Right => $crate::Action::Right,
                NavAction::Confirm => $crate::Action::Confirm,
                NavAction::Back => $crate::Action::Back,
                NavAction::Menu => $crate::Action::Menu,
                NavAction::Search => $crate::Action::Search,
                NavAction::Secondary => $crate::Action::Secondary,
                NavAction::PrevTab => $crate::Action::PrevTab,
                NavAction::NextTab => $crate::Action::NextTab,
                NavAction::Guide => $crate::Action::Guide,
            }
        }

        /// Hint bar content for the `hints` property of a page.
        #[allow(dead_code)]
        fn hint_model(hints: $crate::Hints) -> ::slint::ModelRc<Hint> {
            use $crate::HintButton;
            ::slint::ModelRc::new(::slint::VecModel::from(
                hints
                    .into_vec()
                    .into_iter()
                    .map(|hint| Hint {
                        button: match hint.button {
                            HintButton::Dpad => GlyphButton::Dpad,
                            HintButton::UpDown => GlyphButton::Updown,
                            HintButton::Confirm => GlyphButton::Confirm,
                            HintButton::Back => GlyphButton::Back,
                            HintButton::North => GlyphButton::North,
                            HintButton::West => GlyphButton::West,
                            HintButton::Menu => GlyphButton::Menu,
                            HintButton::Guide => GlyphButton::Guide,
                            HintButton::L1 => GlyphButton::L1,
                            HintButton::R1 => GlyphButton::R1,
                            HintButton::Quit => GlyphButton::Quit,
                        },
                        label: hint.label.into(),
                    })
                    .collect::<Vec<_>>(),
            ))
        }

        /// The style for the `style` property of `HintBar`.
        #[allow(dead_code)]
        fn controller_style(glyphs: Option<$crate::GlyphStyle>) -> ControllerStyle {
            match glyphs {
                Some($crate::GlyphStyle::PlayStation) => ControllerStyle::Playstation,
                Some($crate::GlyphStyle::Xbox) => ControllerStyle::Xbox,
                None => ControllerStyle::Keyboard,
            }
        }

        /// Accent color and animation durations of the user's profile into the `Theme` global.
        #[allow(dead_code)]
        fn apply_theme(theme: Theme<'_>, session: &$crate::Session) {
            let [r, g, b] = session.accent_rgb();
            let durations = session.durations();
            theme.set_accent(::slint::Color::from_rgb_u8(r, g, b));
            theme.set_focus_duration(durations[0]);
            theme.set_background_duration(durations[1]);
            theme.set_pulse_duration(durations[2].max(1));
            theme.set_launch_duration(durations[3]);
            theme.set_page_duration(durations[4]);
        }
    };
}
