//! Pointer hit testing against CPU-side alpha masks.
//!
//! Transparent pixels of a widget must not take hover or clicks from whatever is behind
//! it, so hit testing samples the widget's shape mask (never the GPU) and falls through
//! to lower widgets when the sampled alpha is below the threshold.

use super::geometry::NineSliceLayout;

/// Default alpha (0–255) at or above which a pixel counts as part of the shape.
pub const DEFAULT_ALPHA_THRESHOLD: u8 = 128;

/// Single-channel shape mask kept on the CPU for hit testing.
#[derive(Debug, Clone)]
pub struct AlphaMask {
    width: u32,
    height: u32,
    alpha: Vec<u8>,
}

impl AlphaMask {
    /// Extracts the alpha channel of straight-alpha RGBA8 pixels.
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Self {
        let alpha = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| pixel[3])
            .collect();
        Self {
            width,
            height,
            alpha,
        }
    }

    /// Alpha of the texel containing `(x, y)` (texel coordinates, clamped to the image).
    pub fn alpha_at(&self, x: f32, y: f32) -> u8 {
        if self.width == 0 || self.height == 0 {
            return 0;
        }
        let column = (x.max(0.0) as u32).min(self.width - 1);
        let row = (y.max(0.0) as u32).min(self.height - 1);
        self.alpha
            .get((row * self.width + column) as usize)
            .copied()
            .unwrap_or(0)
    }
}

/// One hit-testable widget, listed in draw order (later entries are on top).
#[derive(Debug, Clone, Copy)]
pub struct HitTarget<'a> {
    pub layout: &'a NineSliceLayout,
    pub mask: &'a AlphaMask,
    /// Disabled or otherwise non-interactive widgets still block the pointer.
    pub interactive: bool,
}

/// Result of a hit test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// No widget's shape is under the pointer.
    Nothing,
    /// The topmost shape under the pointer belongs to a non-interactive widget.
    Blocked(usize),
    /// The topmost shape under the pointer belongs to an interactive widget.
    Interactive(usize),
}

/// Finds the topmost widget whose shape covers `(x, y)`. `targets` are in draw order;
/// the returned index refers to that order.
pub fn topmost_hit<'a, I>(targets: I, x: f32, y: f32, threshold: u8) -> Hit
where
    I: IntoIterator<Item = HitTarget<'a>>,
    I::IntoIter: DoubleEndedIterator + ExactSizeIterator,
{
    for (index, target) in targets.into_iter().enumerate().rev() {
        let Some([u, v]) = target.layout.texel_at(x, y) else {
            continue;
        };
        if target.mask.alpha_at(u, v) < threshold {
            continue; // Transparent pixel: fall through to widgets behind.
        }
        return if target.interactive {
            Hit::Interactive(index)
        } else {
            Hit::Blocked(index)
        };
    }
    Hit::Nothing
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ui::geometry::{NineSlice, Rect};

    const NO_BORDERS: NineSlice = NineSlice {
        left: 0.0,
        right: 0.0,
        top: 0.0,
        bottom: 0.0,
    };

    /// 4x2 mask: left column transparent, the rest opaque.
    fn mask() -> AlphaMask {
        let mut rgba = Vec::new();
        for _row in 0..2 {
            for column in 0..4 {
                let alpha = if column == 0 { 0 } else { 255 };
                rgba.extend_from_slice(&[255, 255, 255, alpha]);
            }
        }
        AlphaMask::from_rgba(4, 2, &rgba)
    }

    fn layout(x: f32) -> NineSliceLayout {
        NineSliceLayout::new(Rect::new(x, 0.0, 40.0, 20.0), [4.0, 2.0], NO_BORDERS)
    }

    #[test]
    fn samples_and_clamps_alpha() {
        let mask = mask();
        assert_eq!(mask.alpha_at(0.5, 0.5), 0);
        assert_eq!(mask.alpha_at(1.5, 1.5), 255);
        assert_eq!(mask.alpha_at(-3.0, 9.0), 0);
        assert_eq!(mask.alpha_at(99.0, 0.0), 255);
    }

    #[test]
    fn opaque_pixel_hits_and_transparent_pixel_falls_through() {
        let mask = mask();
        let back = layout(0.0);
        let front = layout(20.0); // Overlaps `back` on x in 20..40.
        let targets = [
            HitTarget {
                layout: &back,
                mask: &mask,
                interactive: true,
            },
            HitTarget {
                layout: &front,
                mask: &mask,
                interactive: true,
            },
        ];
        // x = 25 is the transparent left column of `front`: the click goes to `back`.
        assert_eq!(topmost_hit(targets, 25.0, 5.0, 128), Hit::Interactive(0));
        // x = 35 is opaque in `front`, which is on top.
        assert_eq!(topmost_hit(targets, 35.0, 5.0, 128), Hit::Interactive(1));
        // x = 5 is the transparent column of `back`, nothing else there.
        assert_eq!(topmost_hit(targets, 5.0, 5.0, 128), Hit::Nothing);
    }

    #[test]
    fn disabled_widget_blocks_widgets_behind() {
        let mask = mask();
        let back = layout(0.0);
        let front = layout(20.0);
        let targets = [
            HitTarget {
                layout: &back,
                mask: &mask,
                interactive: true,
            },
            HitTarget {
                layout: &front,
                mask: &mask,
                interactive: false,
            },
        ];
        assert_eq!(topmost_hit(targets, 35.0, 5.0, 128), Hit::Blocked(1));
    }
}
