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

    /// The widget that captured the pointer on press (drags keep going outside it).
    pub fn captured(&self) -> Option<usize> {
        self.pressed
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

/// Where the pointer is over the window (physical pixels), robust to unreliable
/// enter / leave reports.
///
/// Windows reports a leave whenever the window changes under a resting cursor (e.g. a
/// switch to exclusive fullscreen), and winit then sends no move on re-entry while the
/// position is unchanged. Remembering the last position keeps the pointer usable: an
/// enter without a move means "back where it was", and a button press proves the
/// pointer is over the window.
#[derive(Debug, Default, Clone, Copy)]
pub struct PointerPosition {
    current: Option<[f32; 2]>,
    last: Option<[f32; 2]>,
}

impl PointerPosition {
    pub fn get(&self) -> Option<[f32; 2]> {
        self.current
    }

    pub fn moved(&mut self, position: [f32; 2]) {
        self.current = Some(position);
        self.last = Some(position);
    }

    /// Hover stops; the position is remembered for [`Self::entered`] / [`Self::pressed`].
    pub fn left(&mut self) {
        self.current = None;
    }

    /// Re-entered without a move event: the pointer is where it was last seen.
    pub fn entered(&mut self) {
        self.current = self.last;
    }

    /// A button went down on the window, so the pointer is over it. Returns whether the
    /// position had to be restored (hover must be re-evaluated).
    pub fn pressed(&mut self) -> bool {
        if self.current.is_some() || self.last.is_none() {
            return false;
        }
        self.current = self.last;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_survives_leave_reports() {
        let mut pointer = PointerPosition::default();
        assert!(!pointer.pressed(), "nothing known yet");
        pointer.moved([10.0, 20.0]);
        pointer.left();
        assert_eq!(pointer.get(), None, "no hover after leaving");
        // A mode switch under a resting cursor: enter without a move.
        pointer.entered();
        assert_eq!(pointer.get(), Some([10.0, 20.0]));
        // A spurious leave right before a click: the press restores the position.
        pointer.left();
        assert!(pointer.pressed());
        assert_eq!(pointer.get(), Some([10.0, 20.0]));
        assert!(!pointer.pressed(), "already known");
        pointer.moved([30.0, 40.0]);
        assert_eq!(pointer.get(), Some([30.0, 40.0]));
    }

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
        assert_eq!(
            router.captured(),
            Some(1),
            "capture survives leaving the widget"
        );
        assert_eq!(router.release(), None);
        assert_eq!(router.captured(), None);
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
