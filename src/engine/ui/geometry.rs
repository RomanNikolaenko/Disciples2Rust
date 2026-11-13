//! UI geometry in physical screen pixels: rectangles and 9-slice mapping.
//!
//! Coordinate space: origin at the top-left of the surface, x right, y down, units are
//! physical pixels. Texture coordinates are in texels of the source image unless stated
//! otherwise.

/// Axis-aligned rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// Half-open containment: the right and bottom edges are outside.
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w && py < self.y + self.h
    }
}

/// Fixed border widths of a 9-slice image, in source texels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NineSlice {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

/// A 9-slice image placed into a destination rectangle.
///
/// Borders are scaled by `rect.h / texture_height`, so corners keep their aspect ratio
/// and the image is never distorted at the edges; only the middle row/column stretch.
/// If the scaled borders do not fit, they shrink proportionally.
///
/// The same piecewise mapping is used for vertex generation and for pointer hit testing,
/// so the hit shape always matches what is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NineSliceLayout {
    rect: Rect,
    texture_size: [f32; 2],
    /// Destination stops along x / y (pixels).
    xs: [f32; 4],
    ys: [f32; 4],
    /// Source stops along x / y (texels).
    us: [f32; 4],
    vs: [f32; 4],
}

impl NineSliceLayout {
    pub fn new(rect: Rect, texture_size: [f32; 2], borders: NineSlice) -> Self {
        let scale = if texture_size[1] > 0.0 {
            rect.h / texture_size[1]
        } else {
            1.0
        };
        let (xs, us) = axis_stops(
            rect.x,
            rect.w,
            texture_size[0],
            borders.left,
            borders.right,
            scale,
        );
        let (ys, vs) = axis_stops(
            rect.y,
            rect.h,
            texture_size[1],
            borders.top,
            borders.bottom,
            scale,
        );
        Self {
            rect,
            texture_size,
            xs,
            ys,
            us,
            vs,
        }
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// Grid vertex `(column, row)` with `column, row` in `0..4`: its position in pixels
    /// and its normalized texture coordinate.
    pub fn grid_vertex(&self, column: usize, row: usize) -> ([f32; 2], [f32; 2]) {
        let uv = [
            self.us[column] / self.texture_size[0],
            self.vs[row] / self.texture_size[1],
        ];
        ([self.xs[column], self.ys[row]], uv)
    }

    /// Maps a pixel position to the source texel it samples, or `None` outside the rect.
    pub fn texel_at(&self, px: f32, py: f32) -> Option<[f32; 2]> {
        if !self.rect.contains(px, py) {
            return None;
        }
        Some([
            map_through(&self.xs, &self.us, px),
            map_through(&self.ys, &self.vs, py),
        ])
    }
}

/// Destination and source stops along one axis.
fn axis_stops(
    origin: f32,
    dest_len: f32,
    src_len: f32,
    border_a: f32,
    border_b: f32,
    scale: f32,
) -> ([f32; 4], [f32; 4]) {
    let mut a = border_a * scale;
    let mut b = border_b * scale;
    let total = a + b;
    if total > dest_len && total > 0.0 {
        let shrink = dest_len / total;
        a *= shrink;
        b *= shrink;
    }
    (
        [origin, origin + a, origin + dest_len - b, origin + dest_len],
        [0.0, border_a, src_len - border_b, src_len],
    )
}

/// Piecewise-linear mapping through matching stop arrays.
fn map_through(dest: &[f32; 4], src: &[f32; 4], value: f32) -> f32 {
    let segment = (0..3).find(|&i| value <= dest[i + 1]).unwrap_or(2);
    let span = dest[segment + 1] - dest[segment];
    let t = if span > 0.0 {
        ((value - dest[segment]) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };
    src[segment] + t * (src[segment + 1] - src[segment])
}

/// Normalized texture rectangle that makes a texture of `texture_size` cover a viewport
/// of `viewport_size` without distortion (cropping the overflowing axis, centered).
pub fn cover_uv(texture_size: [f32; 2], viewport_size: [f32; 2]) -> Rect {
    let texture_aspect = texture_size[0] / texture_size[1];
    let viewport_aspect = viewport_size[0] / viewport_size[1];
    if !texture_aspect.is_finite() || !viewport_aspect.is_finite() {
        return Rect::new(0.0, 0.0, 1.0, 1.0);
    }
    if viewport_aspect > texture_aspect {
        let visible = texture_aspect / viewport_aspect;
        Rect::new(0.0, (1.0 - visible) / 2.0, 1.0, visible)
    } else {
        let visible = viewport_aspect / texture_aspect;
        Rect::new((1.0 - visible) / 2.0, 0.0, visible, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BORDERS: NineSlice = NineSlice {
        left: 40.0,
        right: 40.0,
        top: 32.0,
        bottom: 32.0,
    };

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn rect_containment_is_half_open() {
        let rect = Rect::new(10.0, 20.0, 30.0, 40.0);
        assert!(rect.contains(10.0, 20.0));
        assert!(rect.contains(39.9, 59.9));
        assert!(!rect.contains(40.0, 30.0));
        assert!(!rect.contains(15.0, 60.0));
    }

    #[test]
    fn corners_keep_scale_and_center_stretches() {
        // 512x128 texture drawn at half height: borders scale by 0.5.
        let layout =
            NineSliceLayout::new(Rect::new(0.0, 0.0, 256.0, 64.0), [512.0, 128.0], BORDERS);
        let [u, _] = layout.texel_at(10.0, 1.0).unwrap();
        assert!(approx(u, 20.0), "left border maps 1:2, got {u}");
        let [u, _] = layout.texel_at(128.0, 1.0).unwrap();
        assert!(approx(u, 256.0), "center maps to texture center, got {u}");
        let [u, v] = layout.texel_at(246.0, 63.0).unwrap();
        assert!(approx(u, 492.0), "right border, got {u}");
        assert!(approx(v, 126.0), "bottom border, got {v}");
    }

    #[test]
    fn grid_matches_hit_mapping() {
        let layout =
            NineSliceLayout::new(Rect::new(100.0, 50.0, 300.0, 64.0), [512.0, 128.0], BORDERS);
        let (pos, uv) = layout.grid_vertex(1, 1);
        assert!(approx(pos[0], 120.0) && approx(pos[1], 66.0));
        assert!(approx(uv[0], 40.0 / 512.0) && approx(uv[1], 32.0 / 128.0));
        let (pos, uv) = layout.grid_vertex(3, 3);
        assert!(approx(pos[0], 400.0) && approx(pos[1], 114.0));
        assert!(approx(uv[0], 1.0) && approx(uv[1], 1.0));
    }

    #[test]
    fn borders_shrink_when_rect_is_too_narrow() {
        // Scaled borders are 40 + 40 = 80 px but only 40 px are available.
        let layout =
            NineSliceLayout::new(Rect::new(0.0, 0.0, 40.0, 128.0), [512.0, 128.0], BORDERS);
        let (pos, _) = layout.grid_vertex(1, 0);
        assert!(approx(pos[0], 20.0));
        let (pos, _) = layout.grid_vertex(2, 0);
        assert!(approx(pos[0], 20.0));
        let [u, _] = layout.texel_at(30.0, 5.0).unwrap();
        assert!(
            approx(u, 492.0),
            "right half samples the right border, got {u}"
        );
    }

    #[test]
    fn outside_rect_has_no_texel() {
        let layout =
            NineSliceLayout::new(Rect::new(0.0, 0.0, 100.0, 50.0), [512.0, 128.0], BORDERS);
        assert_eq!(layout.texel_at(-1.0, 10.0), None);
        assert_eq!(layout.texel_at(100.0, 10.0), None);
    }

    #[test]
    fn cover_crops_the_overflowing_axis() {
        // 16:9 texture on a 4:3 viewport: crop horizontally.
        let uv = cover_uv([1920.0, 1080.0], [1024.0, 768.0]);
        assert!(approx(uv.y, 0.0) && approx(uv.h, 1.0));
        assert!(approx(uv.w, 0.75) && approx(uv.x, 0.125));
        // 16:9 texture on a 21:9 viewport: crop vertically.
        let uv = cover_uv([1920.0, 1080.0], [2560.0, 1080.0]);
        assert!(approx(uv.x, 0.0) && approx(uv.w, 1.0));
        assert!(approx(uv.h, 0.75) && approx(uv.y, 0.125));
    }
}
