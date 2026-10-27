//! Button hints for the hint bar, independent of Slint: the app builds them here and converts them
//! to its generated `Hint` type with `slint_bridge!`.

/// Which button a hint shows; the glyph depends on the controller style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintButton {
    Dpad,
    UpDown,
    Confirm,
    Back,
    North,
    West,
    Menu,
    Guide,
    L1,
    R1,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hint {
    pub button: HintButton,
    pub label: String,
}

/// Builder for the hints of a screen: `Hints::new().dpad("Select").confirm("OK").back("Back")`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hints(Vec<Hint>);

macro_rules! hint_methods {
    ($($name:ident => $button:ident),* $(,)?) => {
        $(
            pub fn $name(self, label: impl Into<String>) -> Self {
                self.with(HintButton::$button, label)
            }
        )*
    };
}

impl Hints {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, button: HintButton, label: impl Into<String>) -> Self {
        self.0.push(Hint { button, label: label.into() });
        self
    }

    hint_methods! {
        dpad => Dpad,
        updown => UpDown,
        confirm => Confirm,
        back => Back,
        north => North,
        west => West,
        menu => Menu,
        l1 => L1,
        r1 => R1,
    }

    pub fn into_vec(self) -> Vec<Hint> {
        self.0
    }

    pub fn as_slice(&self) -> &[Hint] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_keeps_the_order() {
        let hints = Hints::new().dpad("Select").confirm("OK").back("Back");
        let buttons: Vec<_> = hints.as_slice().iter().map(|h| h.button).collect();
        assert_eq!(buttons, [HintButton::Dpad, HintButton::Confirm, HintButton::Back]);
        assert_eq!(hints.as_slice()[1].label, "OK");
    }
}
