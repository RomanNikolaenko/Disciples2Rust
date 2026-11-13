//! Tweening of scalar values (hover amounts, opacity, …).

/// Easing curves mapping linear progress `t` in `[0, 1]` to eased progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Easing {
    Linear,
    EaseOutCubic,
    EaseInOutCubic,
}

impl Easing {
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::EaseOutCubic => 1.0 - (1.0 - t).powi(3),
            Self::EaseInOutCubic => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
        }
    }
}

/// A scalar animated towards a target value.
///
/// Retargeting starts from the value currently displayed, so interrupted or reversed
/// animations (rapid hover in/out, menu toggles) never jump.
#[derive(Debug, Clone, Copy)]
pub struct Tween {
    from: f32,
    to: f32,
    elapsed: f32,
    duration: f32,
    easing: Easing,
}

impl Tween {
    /// A settled tween resting at `value`.
    pub fn new(value: f32) -> Self {
        Self {
            from: value,
            to: value,
            elapsed: 0.0,
            duration: 0.0,
            easing: Easing::Linear,
        }
    }

    pub fn value(&self) -> f32 {
        if self.elapsed >= self.duration {
            return self.to;
        }
        let progress = self.easing.apply(self.elapsed / self.duration);
        self.from + (self.to - self.from) * progress
    }

    pub fn is_settled(&self) -> bool {
        self.elapsed >= self.duration
    }

    /// Starts animating from the current value to `target` over `duration` seconds.
    /// Requesting the target that is already being approached keeps the running
    /// animation instead of restarting it.
    pub fn animate_to(&mut self, target: f32, duration: f32, easing: Easing) {
        if target == self.to {
            return;
        }
        self.from = self.value();
        self.to = target;
        self.elapsed = 0.0;
        self.duration = duration.max(0.0);
        self.easing = easing;
    }

    /// Jumps to `value` without animating.
    pub fn set(&mut self, value: f32) {
        *self = Self::new(value);
    }

    /// Advances the animation by `dt` seconds.
    pub fn update(&mut self, dt: f32) {
        if !self.is_settled() {
            self.elapsed = (self.elapsed + dt.max(0.0)).min(self.duration);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-5;

    #[test]
    fn easing_endpoints_are_fixed() {
        for easing in [Easing::Linear, Easing::EaseOutCubic, Easing::EaseInOutCubic] {
            assert!(easing.apply(0.0).abs() < EPS, "{easing:?}");
            assert!((easing.apply(1.0) - 1.0).abs() < EPS, "{easing:?}");
        }
        assert!((Easing::EaseInOutCubic.apply(0.5) - 0.5).abs() < EPS);
        assert!(Easing::EaseOutCubic.apply(0.5) > 0.5);
    }

    #[test]
    fn reaches_target_after_duration() {
        let mut tween = Tween::new(0.0);
        tween.animate_to(1.0, 0.2, Easing::Linear);
        tween.update(0.1);
        assert!((tween.value() - 0.5).abs() < EPS);
        assert!(!tween.is_settled());
        tween.update(0.5);
        assert_eq!(tween.value(), 1.0);
        assert!(tween.is_settled());
    }

    #[test]
    fn reversal_starts_from_the_displayed_value() {
        let mut tween = Tween::new(0.0);
        tween.animate_to(1.0, 0.2, Easing::EaseOutCubic);
        tween.update(0.07);
        let before = tween.value();
        tween.animate_to(0.0, 0.3, Easing::EaseOutCubic);
        assert!((tween.value() - before).abs() < EPS, "no jump on reversal");
        tween.update(0.3);
        assert_eq!(tween.value(), 0.0);
    }

    #[test]
    fn same_target_does_not_restart() {
        let mut tween = Tween::new(0.0);
        tween.animate_to(1.0, 1.0, Easing::Linear);
        tween.update(0.5);
        tween.animate_to(1.0, 1.0, Easing::Linear);
        assert!((tween.value() - 0.5).abs() < EPS);
    }

    #[test]
    fn zero_duration_jumps() {
        let mut tween = Tween::new(0.25);
        tween.animate_to(0.75, 0.0, Easing::Linear);
        assert_eq!(tween.value(), 0.75);
        assert!(tween.is_settled());
        tween.set(0.1);
        assert_eq!(tween.value(), 0.1);
        assert!(tween.is_settled());
    }
}
