//! Fade-out → switch → fade-in sequencing between menu screens.

use crate::engine::animation::{Easing, Tween};

const STARTUP_FADE_IN: f32 = 0.8;
const FADE_OUT: f32 = 0.22;
const FADE_IN: f32 = 0.4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition<S> {
    To(S),
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowEvent {
    None,
    /// The current screen just changed (the new one starts fading in).
    ScreenChanged,
    /// The exit fade-out has finished.
    ExitReady,
}

/// Which screen is shown and the opacity of the whole menu group.
///
/// A transition first fades the group out (starting from whatever opacity is displayed,
/// so interrupting a fade-in never jumps), switches the screen, then fades back in.
/// Input is ignored while a transition is running.
#[derive(Debug, Clone, Copy)]
pub struct ScreenFlow<S> {
    current: S,
    transition: Option<Transition<S>>,
    opacity: Tween,
}

impl<S: Copy> ScreenFlow<S> {
    /// Starts on `screen`, fading in from fully transparent.
    pub fn new(screen: S) -> Self {
        let mut opacity = Tween::new(0.0);
        opacity.animate_to(1.0, STARTUP_FADE_IN, Easing::EaseOutCubic);
        Self {
            current: screen,
            transition: None,
            opacity,
        }
    }

    pub fn current(&self) -> S {
        self.current
    }

    pub fn opacity(&self) -> f32 {
        self.opacity.value()
    }

    pub fn is_interactive(&self) -> bool {
        self.transition.is_none()
    }

    /// Starts a transition unless one is already running.
    pub fn request(&mut self, transition: Transition<S>) {
        if self.transition.is_some() {
            return;
        }
        self.transition = Some(transition);
        self.opacity
            .animate_to(0.0, FADE_OUT, Easing::EaseInOutCubic);
    }

    pub fn update(&mut self, dt: f32) -> FlowEvent {
        self.opacity.update(dt);
        let Some(transition) = self.transition else {
            return FlowEvent::None;
        };
        if !self.opacity.is_settled() || self.opacity.value() > 0.0 {
            return FlowEvent::None;
        }
        match transition {
            Transition::To(screen) => {
                self.current = screen;
                self.transition = None;
                self.opacity.animate_to(1.0, FADE_IN, Easing::EaseOutCubic);
                FlowEvent::ScreenChanged
            }
            // Stays in the exit transition: the menu remains non-interactive.
            Transition::Exit => FlowEvent::ExitReady,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Screen {
        A,
        B,
    }

    fn run(flow: &mut ScreenFlow<Screen>, seconds: f32) -> Vec<FlowEvent> {
        let mut events = Vec::new();
        let mut elapsed = 0.0;
        while elapsed < seconds {
            let event = flow.update(0.01);
            if event != FlowEvent::None {
                events.push(event);
            }
            elapsed += 0.01;
        }
        events
    }

    #[test]
    fn starts_by_fading_in() {
        let mut flow = ScreenFlow::new(Screen::A);
        assert_eq!(flow.opacity(), 0.0);
        assert!(flow.is_interactive());
        run(&mut flow, 1.0);
        assert_eq!(flow.opacity(), 1.0);
    }

    #[test]
    fn switch_fades_out_then_in() {
        let mut flow = ScreenFlow::new(Screen::A);
        run(&mut flow, 1.0);
        flow.request(Transition::To(Screen::B));
        assert!(!flow.is_interactive());
        flow.update(0.1);
        assert_eq!(flow.current(), Screen::A, "still fading out");
        assert!(flow.opacity() < 1.0);
        let events = run(&mut flow, 1.0);
        assert_eq!(events, [FlowEvent::ScreenChanged]);
        assert_eq!(flow.current(), Screen::B);
        assert_eq!(flow.opacity(), 1.0);
        assert!(flow.is_interactive());
    }

    #[test]
    fn interrupting_the_startup_fade_does_not_jump() {
        let mut flow = ScreenFlow::new(Screen::A);
        flow.update(0.2);
        let before = flow.opacity();
        assert!(before > 0.0 && before < 1.0);
        flow.request(Transition::To(Screen::B));
        assert!((flow.opacity() - before).abs() < 1e-5);
    }

    #[test]
    fn requests_during_a_transition_are_ignored() {
        let mut flow = ScreenFlow::new(Screen::A);
        run(&mut flow, 1.0);
        flow.request(Transition::To(Screen::B));
        flow.request(Transition::Exit);
        let events = run(&mut flow, 1.0);
        assert_eq!(events, [FlowEvent::ScreenChanged]);
        assert_eq!(flow.current(), Screen::B);
    }

    #[test]
    fn exit_reports_ready_after_fading_out() {
        let mut flow = ScreenFlow::new(Screen::A);
        run(&mut flow, 1.0);
        flow.request(Transition::Exit);
        assert_eq!(flow.update(0.05), FlowEvent::None);
        let events = run(&mut flow, 0.5);
        assert!(events.contains(&FlowEvent::ExitReady));
        assert_eq!(flow.opacity(), 0.0);
        assert!(!flow.is_interactive());
    }
}
