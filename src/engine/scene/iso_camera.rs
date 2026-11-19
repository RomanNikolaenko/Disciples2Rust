//! An isometric-style camera over a flat, bounded world: orthographic, looking down at
//! [`PITCH`] from the diagonal ([`YAW`]), so a square map shows as a diamond.
//!
//! Coordinate spaces:
//! - **map**: 2D world points `(x, y)` in tiles, origin at the map's top-left corner, y
//!   towards the map's south; the map spans `0..size`;
//! - **world**: 3D, +Y up; map point `(x, y)` is world `(x, 0, y)` on the ground;
//! - **screen**: physical pixels of the viewport, origin top-left, y down.
//!
//! The camera looks from above the map's south-east, so the map's north-west corner is
//! at the top of the screen and its south-east corner at the bottom. `zoom` is screen
//! pixels per world unit across the view; the center (a map point) is kept on the map.

use std::f32::consts::FRAC_PI_4;

use glam::camera::rh::{proj::directx, view};
use glam::{Mat4, Vec2, Vec3, Vec4Swizzles as _};

/// Turn about +Y of the direction the camera looks from: from the map's south-east.
pub const YAW: f32 = FRAC_PI_4;
/// Angle between the ground and the direction the camera looks along.
pub const PITCH: f32 = 0.62; // ~35.5°
/// How far the eye stands back from the center, and the depth range around it (world
/// units); generous for maps of a few hundred tiles.
const EYE_DISTANCE: f32 = 800.0;
const NEAR: f32 = 1.0;
const FAR: f32 = 1600.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IsoCamera {
    /// Map point at the viewport's center.
    center: Vec2,
    /// Screen pixels per world unit.
    zoom: f32,
    viewport: Vec2,
    map: Vec2,
    min_zoom: f32,
    max_zoom: f32,
}

impl IsoCamera {
    /// A camera centered on the map at `zoom`, kept within `min_zoom..=max_zoom`.
    pub fn new(map: Vec2, viewport: Vec2, zoom: f32, min_zoom: f32, max_zoom: f32) -> Self {
        let mut camera = Self {
            center: map / 2.0,
            zoom,
            viewport,
            map,
            min_zoom: min_zoom.max(1e-3),
            max_zoom: max_zoom.max(min_zoom),
        };
        camera.zoom = camera.clamp_zoom_value(zoom);
        camera
    }

    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    pub fn viewport(&self) -> Vec2 {
        self.viewport
    }

    /// Clamps `zoom` to the allowed range.
    pub fn clamp_zoom_value(&self, zoom: f32) -> f32 {
        if zoom.is_finite() {
            zoom.clamp(self.min_zoom, self.max_zoom)
        } else {
            self.min_zoom
        }
    }

    /// Unit vector from the center towards the eye.
    pub fn eye_direction() -> Vec3 {
        Vec3::new(
            YAW.sin() * PITCH.cos(),
            PITCH.sin(),
            YAW.cos() * PITCH.cos(),
        )
    }

    /// Turn about +Y that makes a model's front (+Z) face the viewer.
    pub fn facing_viewer() -> f32 {
        YAW
    }

    /// World → view (right-handed: x right, y up, the camera looking along -z).
    pub fn view(&self) -> Mat4 {
        let target = Vec3::new(self.center.x, 0.0, self.center.y);
        view::look_at_mat4(
            target + Self::eye_direction() * EYE_DISTANCE,
            target,
            Vec3::Y,
        )
    }

    /// The view's rotation alone: world → the lighting frame (+Z towards the viewer).
    pub fn view_rotation(&self) -> Mat4 {
        Self::rotation()
    }

    /// The view's rotation, the same wherever the camera looks (it never turns): world →
    /// the lighting frame.
    pub fn rotation() -> Mat4 {
        let mut rotation = view::look_at_mat4(Self::eye_direction(), Vec3::ZERO, Vec3::Y);
        rotation.w_axis = glam::Vec4::W;
        rotation
    }

    /// World → clip space (wgpu depth 0..1).
    pub fn clip_from_world(&self) -> Mat4 {
        let half = self.viewport.max(Vec2::ONE) / (2.0 * self.zoom);
        // wgpu's clip space is Direct3D's: depth 0..1, y up.
        directx::orthographic(-half.x, half.x, -half.y, half.y, NEAR, FAR) * self.view()
    }

    /// Screen position of world point `world`.
    pub fn point_to_screen(&self, world: Vec3) -> Vec2 {
        let clip = self.clip_from_world() * world.extend(1.0);
        let ndc = clip.xy() / clip.w;
        Vec2::new(
            (ndc.x + 1.0) / 2.0 * self.viewport.x,
            (1.0 - ndc.y) / 2.0 * self.viewport.y,
        )
    }

    /// The line of sight under screen position `screen`: a point on the camera's plane
    /// (far above the map) and the unit direction the camera looks along, down.
    pub fn screen_ray(&self, screen: Vec2) -> (Vec3, Vec3) {
        let size = self.viewport.max(Vec2::ONE);
        let ndc = Vec2::new(screen.x / size.x * 2.0 - 1.0, 1.0 - screen.y / size.y * 2.0);
        let half = size / (2.0 * self.zoom);
        let world_from_view = self.view().inverse();
        let origin =
            world_from_view.transform_point3(Vec3::new(ndc.x * half.x, ndc.y * half.y, 0.0));
        (origin, -Self::eye_direction())
    }

    /// The map point at height 0 under screen position `screen`.
    pub fn screen_to_world(&self, screen: Vec2) -> Vec2 {
        let (origin, forward) = self.screen_ray(screen);
        let ground = origin + forward * (-origin.y / forward.y);
        Vec2::new(ground.x, ground.z)
    }

    /// Centers the view on map point `map`, kept on the map.
    pub fn look_at(&mut self, map: Vec2) {
        if map.is_finite() {
            self.center = map;
            self.clamp_center();
        }
    }

    /// Moves the view so the ground follows a pointer drag of `delta` screen pixels.
    pub fn pan_screen(&mut self, delta: Vec2) {
        let middle = self.viewport / 2.0;
        let moved = self.screen_to_world(middle + delta) - self.screen_to_world(middle);
        self.center -= moved;
        self.clamp_center();
    }

    /// Sets the zoom while the map point under `anchor` (screen pixels) stays put, as
    /// far as the map's edges allow.
    pub fn zoom_at(&mut self, anchor: Vec2, zoom: f32) {
        let fixed = self.screen_to_world(anchor);
        self.zoom = self.clamp_zoom_value(zoom);
        self.center += fixed - self.screen_to_world(anchor);
        self.clamp_center();
    }

    /// The viewport changed size (physical pixels).
    pub fn set_viewport(&mut self, viewport: Vec2) {
        self.viewport = viewport;
    }

    fn clamp_center(&mut self) {
        self.center = self.center.clamp(Vec2::ZERO, self.map.max(Vec2::ZERO));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl IsoCamera {
        /// Screen position of map point `map` (at height 0).
        fn map_to_screen(&self, map: Vec2) -> Vec2 {
            self.point_to_screen(Vec3::new(map.x, 0.0, map.y))
        }
    }

    const MAP: Vec2 = Vec2::new(120.0, 120.0);
    const VIEW: Vec2 = Vec2::new(1920.0, 1080.0);

    fn camera() -> IsoCamera {
        IsoCamera::new(MAP, VIEW, 48.0, 20.0, 96.0)
    }

    #[test]
    fn the_center_is_mid_screen_and_points_round_trip() {
        let camera = camera();
        assert!(
            camera
                .map_to_screen(MAP / 2.0)
                .abs_diff_eq(VIEW / 2.0, 1e-2)
        );
        for point in [
            Vec2::new(10.0, 80.0),
            Vec2::new(60.5, 59.0),
            Vec2::new(100.0, 3.0),
        ] {
            let back = camera.screen_to_world(camera.map_to_screen(point));
            assert!(back.abs_diff_eq(point, 1e-3), "{point} -> {back}");
        }
    }

    #[test]
    fn the_map_shows_as_a_diamond_north_west_up() {
        let mut camera = camera();
        camera.zoom_at(VIEW / 2.0, 20.0);
        let screen = |x: f32, y: f32| camera.map_to_screen(Vec2::new(x, y));
        let (north_west, south_east) = (screen(0.0, 0.0), screen(120.0, 120.0));
        let (north_east, south_west) = (screen(120.0, 0.0), screen(0.0, 120.0));
        assert!(north_west.y < north_east.y && north_west.y < south_west.y);
        assert!(south_east.y > north_east.y && south_east.y > south_west.y);
        assert!(north_east.x > south_west.x);
        // Left and right corners level; top and bottom on the center line.
        assert!((north_east.y - south_west.y).abs() < 1e-2);
        assert!((north_west.x - south_east.x).abs() < 1e-2);
        // Foreshortened: the diamond is wider than tall by 1 / sin(pitch).
        let ratio = (north_east.x - south_west.x) / (south_east.y - north_west.y);
        assert!((ratio - 1.0 / PITCH.sin()).abs() < 1e-3, "{ratio}");
    }

    #[test]
    fn zooming_keeps_the_anchor_and_panning_follows_the_drag() {
        let mut camera = camera();
        let anchor = Vec2::new(700.0, 400.0);
        let under = camera.screen_to_world(anchor);
        camera.zoom_at(anchor, 70.0);
        assert_eq!(camera.zoom(), 70.0);
        assert!(camera.screen_to_world(anchor).abs_diff_eq(under, 1e-3));
        // A drag moves the ground with the pointer.
        let grabbed = camera.screen_to_world(anchor);
        camera.pan_screen(Vec2::new(50.0, -30.0));
        let now = camera.map_to_screen(grabbed);
        assert!(
            now.abs_diff_eq(anchor + Vec2::new(50.0, -30.0), 1e-2),
            "{now}"
        );
        // Limits.
        camera.zoom_at(anchor, 1000.0);
        assert_eq!(camera.zoom(), 96.0);
        assert_eq!(camera.clamp_zoom_value(f32::NAN), 20.0);
    }

    #[test]
    fn the_center_stays_on_the_map() {
        let mut camera = camera();
        camera.look_at(Vec2::new(-50.0, 500.0));
        assert!(
            camera
                .screen_to_world(VIEW / 2.0)
                .abs_diff_eq(Vec2::new(0.0, 120.0), 1e-3)
        );
        camera.pan_screen(Vec2::new(-10_000.0, 0.0));
        let center = camera.screen_to_world(VIEW / 2.0);
        assert!(center.x >= -1e-3 && center.x <= 120.001 && center.y <= 120.001);
    }

    #[test]
    fn the_view_rotation_turns_the_eye_direction_towards_the_viewer() {
        let rotation = camera().view_rotation();
        let towards_eye = rotation.transform_vector3(IsoCamera::eye_direction());
        assert!(towards_eye.abs_diff_eq(Vec3::Z, 1e-5), "{towards_eye}");
        // A model turned to face the viewer has its front along the eye's ground direction.
        let front = Mat4::from_rotation_y(IsoCamera::facing_viewer()).transform_vector3(Vec3::Z);
        let eye = IsoCamera::eye_direction();
        assert!(front.abs_diff_eq(Vec3::new(eye.x, 0.0, eye.z).normalize(), 1e-5));
    }
}
