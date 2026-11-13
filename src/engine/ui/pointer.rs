//! Pointer routing: hover, press capture and click detection.

/// Tracks which widget is hovered and which captured the last press.
///
/// A click is reported only when the button is released over the same widget that
/// received the press; dragging off a widget cancels the click.
#[derive(Debug, Default, Clone, Copy)]
pub struct PointerRouter {
    hovered: Option<usize>,
    pressed: Option<usize>,
}

impl PointerRouter {
    /// Sets the interactive widget under the pointer (from hit testing).
    pub fn set_hovered(&mut self, hovered: Option<usize>) {
        self.hovered = hovered;
    }

    pub fn hovered(&self) -> Option<usize> {
        self.hovered
    }

    /// The widget that should look pressed: the captured one, while still hovered.
    pub fn pressed_visual(&self) -> Option<usize> {
        self.pressed
            .filter(|&pressed| self.hovered == Some(pressed))
    }

    pub fn press(&mut self) {
        self.pressed = self.hovered;
    }

    /// Ends a press; returns the clicked widget, if any.
    pub fn release(&mut self) -> Option<usize> {
        let pressed = self.pressed.take()?;
        (self.hovered == Some(pressed)).then_some(pressed)
    }

    /// Forgets hover and capture (e.g. when the screen changes).
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn press_and_release_on_same_widget_clicks() {
        let mut router = PointerRouter::default();
        router.set_hovered(Some(2));
        router.press();
        assert_eq!(router.pressed_visual(), Some(2));
        assert_eq!(router.release(), Some(2));
        assert_eq!(router.pressed_visual(), None);
    }

    #[test]
    fn dragging_off_cancels_the_click() {
        let mut router = PointerRouter::default();
        router.set_hovered(Some(1));
        router.press();
        router.set_hovered(None);
        assert_eq!(router.pressed_visual(), None);
        assert_eq!(router.release(), None);
    }

    #[test]
    fn press_elsewhere_then_release_on_widget_does_not_click() {
        let mut router = PointerRouter::default();
        router.press();
        router.set_hovered(Some(0));
        assert_eq!(router.release(), None);
    }

    #[test]
    fn returning_to_the_pressed_widget_restores_the_click() {
        let mut router = PointerRouter::default();
        router.set_hovered(Some(3));
        router.press();
        router.set_hovered(Some(1));
        router.set_hovered(Some(3));
        assert_eq!(router.release(), Some(3));
    }
}
