//! Graphics options: value types, the per-option settings, and the Overall presets.
//!
//! No 3D scene exists yet, so these values are stored and offered in the menu; the
//! renderer reads them as the matching features are implemented. Options that depend on
//! technology the engine does not have (upscalers) are shown but not selectable.
//!
//! Overall Quality covers the options that scale cost with quality. Personal
//! preferences and resolution controls (Resolution Scale, upscaling, sharpening, Dynamic
//! Resolution, Depth of Field, Motion Blur, Color Grading, Vignette, Film Grain,
//! Chromatic Aberration) keep their values when a preset is chosen.

use serde::{Deserialize, Serialize};

use super::OptionValue;

/// Quality levels, lowest first. `Epic` / `Cinematic` from older files read as Ultra.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Quality {
    Low,
    Medium,
    #[default]
    High,
    #[serde(alias = "Epic", alias = "Cinematic")]
    Ultra,
}

impl OptionValue for Quality {
    const ALL: &'static [Self] = &[Self::Low, Self::Medium, Self::High, Self::Ultra];

    fn label(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::Ultra => "Ultra",
        }
    }
}

/// A quality level that can also be switched off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OptionalQuality {
    Off,
    Low,
    Medium,
    #[default]
    High,
    #[serde(alias = "Epic", alias = "Cinematic")]
    Ultra,
}

impl OptionValue for OptionalQuality {
    const ALL: &'static [Self] = &[Self::Off, Self::Low, Self::Medium, Self::High, Self::Ultra];

    fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::Ultra => "Ultra",
        }
    }
}

impl From<Quality> for OptionalQuality {
    fn from(quality: Quality) -> Self {
        match quality {
            Quality::Low => Self::Low,
            Quality::Medium => Self::Medium,
            Quality::High => Self::High,
            Quality::Ultra => Self::Ultra,
        }
    }
}

/// Off plus three levels (fog, volumetric fog, bloom).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum EffectLevel {
    Off,
    Low,
    #[default]
    Medium,
    High,
}

impl OptionValue for EffectLevel {
    const ALL: &'static [Self] = &[Self::Off, Self::Low, Self::Medium, Self::High];

    fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
        }
    }
}

/// Older files stored a quality level here; it maps to the method that level used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AntiAliasing {
    Off,
    #[serde(alias = "Low")]
    Fxaa,
    #[serde(alias = "Medium")]
    Smaa,
    #[default]
    #[serde(alias = "High", alias = "Epic", alias = "Cinematic")]
    Taa,
}

impl OptionValue for AntiAliasing {
    const ALL: &'static [Self] = &[Self::Off, Self::Fxaa, Self::Smaa, Self::Taa];

    fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Fxaa => "FXAA",
            Self::Smaa => "SMAA",
            Self::Taa => "TAA",
        }
    }
}

/// Upscalers replace TAA rather than running on top of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum UpscalingMethod {
    #[default]
    Off,
    Fsr,
    Dlss,
    Xess,
}

impl UpscalingMethod {
    /// Methods the engine implements. None yet: the row is shown, but locked to Off.
    pub const SUPPORTED: &'static [Self] = &[Self::Off];
}

impl OptionValue for UpscalingMethod {
    const ALL: &'static [Self] = &[Self::Off, Self::Fsr, Self::Dlss, Self::Xess];

    fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Fsr => "AMD FSR",
            Self::Dlss => "NVIDIA DLSS",
            Self::Xess => "Intel XeSS",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum UpscalingQuality {
    NativeAa,
    #[default]
    Quality,
    Balanced,
    Performance,
}

impl OptionValue for UpscalingQuality {
    const ALL: &'static [Self] = &[
        Self::NativeAa,
        Self::Quality,
        Self::Balanced,
        Self::Performance,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::NativeAa => "Native AA",
            Self::Quality => "Quality",
            Self::Balanced => "Balanced",
            Self::Performance => "Performance",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DynamicResolutionTarget {
    Fps30,
    #[default]
    Fps60,
    Fps120,
}

impl OptionValue for DynamicResolutionTarget {
    const ALL: &'static [Self] = &[Self::Fps30, Self::Fps60, Self::Fps120];

    fn label(self) -> &'static str {
        match self {
            Self::Fps30 => "30 FPS",
            Self::Fps60 => "60 FPS",
            Self::Fps120 => "120 FPS",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TextureFiltering {
    Bilinear,
    Trilinear,
    Anisotropic2x,
    Anisotropic4x,
    #[default]
    Anisotropic8x,
    Anisotropic16x,
}

impl OptionValue for TextureFiltering {
    const ALL: &'static [Self] = &[
        Self::Bilinear,
        Self::Trilinear,
        Self::Anisotropic2x,
        Self::Anisotropic4x,
        Self::Anisotropic8x,
        Self::Anisotropic16x,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Bilinear => "Bilinear",
            Self::Trilinear => "Trilinear",
            Self::Anisotropic2x => "Anisotropic 2×",
            Self::Anisotropic4x => "Anisotropic 4×",
            Self::Anisotropic8x => "Anisotropic 8×",
            Self::Anisotropic16x => "Anisotropic 16×",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ShadowDistance {
    Near,
    Medium,
    #[default]
    Far,
}

impl OptionValue for ShadowDistance {
    const ALL: &'static [Self] = &[Self::Near, Self::Medium, Self::Far];

    fn label(self) -> &'static str {
        match self {
            Self::Near => "Near",
            Self::Medium => "Medium",
            Self::Far => "Far",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AmbientOcclusion {
    Off,
    #[default]
    Ssao,
    Gtao,
}

impl OptionValue for AmbientOcclusion {
    const ALL: &'static [Self] = &[Self::Off, Self::Ssao, Self::Gtao];

    fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Ssao => "SSAO",
            Self::Gtao => "GTAO",
        }
    }
}

/// Percent of the output resolution the 3D scene renders at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ResolutionScale(pub u8);

impl Default for ResolutionScale {
    fn default() -> Self {
        Self(100)
    }
}

impl ResolutionScale {
    /// The nearest offered scale.
    pub fn snapped(self) -> Self {
        let wanted = self.0;
        Self::ALL
            .iter()
            .copied()
            .min_by_key(|scale| scale.0.abs_diff(wanted))
            .unwrap_or_default()
    }
}

impl OptionValue for ResolutionScale {
    const ALL: &'static [Self] = &[Self(50), Self(60), Self(70), Self(80), Self(90), Self(100)];

    fn label(self) -> &'static str {
        match self.0 {
            50 => "50%",
            60 => "60%",
            70 => "70%",
            80 => "80%",
            90 => "90%",
            _ => "100%",
        }
    }
}

pub const MAX_SHARPENING: u8 = 100;

/// Every graphics option. Older files used other names for some fields (aliases).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphicsSettings {
    // General.
    pub view_distance: Quality,
    // Rendering & Upscaling.
    pub resolution_scale: ResolutionScale,
    pub anti_aliasing: AntiAliasing,
    pub upscaling_method: UpscalingMethod,
    pub upscaling_quality: UpscalingQuality,
    /// Percent, 0–100.
    pub image_sharpening: u8,
    pub dynamic_resolution: bool,
    pub dynamic_resolution_target: DynamicResolutionTarget,
    // Textures & Materials.
    #[serde(alias = "textures")]
    pub texture_quality: Quality,
    /// Shader complexity of materials (replaces the former Shading option).
    #[serde(alias = "shading")]
    pub material_quality: Quality,
    pub texture_filtering: TextureFiltering,
    // Geometry & Environment.
    #[serde(alias = "landscape")]
    pub terrain_quality: Quality,
    #[serde(alias = "foliage")]
    pub foliage_quality: Quality,
    /// Mesh detail and LOD distances.
    pub geometry_quality: Quality,
    /// Amount of small decorative objects.
    pub object_detail: Quality,
    pub vegetation_density: Quality,
    // Lighting & Shadows.
    #[serde(alias = "shadows")]
    pub shadow_quality: OptionalQuality,
    pub shadow_distance: ShadowDistance,
    pub global_illumination: OptionalQuality,
    pub ambient_occlusion: AmbientOcclusion,
    pub contact_shadows: bool,
    // Reflections & Water.
    #[serde(alias = "reflections")]
    pub reflection_quality: OptionalQuality,
    pub water_quality: Quality,
    pub water_reflections: bool,
    // Atmosphere.
    pub fog_quality: EffectLevel,
    pub volumetric_fog: EffectLevel,
    pub light_shafts: bool,
    pub weather_effects_quality: Quality,
    // Effects & Particles.
    #[serde(alias = "effects")]
    pub effects_quality: Quality,
    pub particle_quality: Quality,
    pub spell_effects_quality: Quality,
    pub particle_density: Quality,
    // Post Processing.
    #[serde(alias = "post_processing")]
    pub post_processing_quality: Quality,
    pub bloom: EffectLevel,
    pub depth_of_field: bool,
    pub motion_blur: bool,
    pub color_grading: bool,
    pub vignette: bool,
    pub film_grain: bool,
    pub chromatic_aberration: bool,
}

impl Default for GraphicsSettings {
    fn default() -> Self {
        Self::preset(Quality::High)
    }
}

/// Picks the value for `level` from one entry per level (Low, Medium, High, Ultra).
fn by_level<T: Copy>(level: Quality, values: [T; 4]) -> T {
    values[level.index()]
}

impl GraphicsSettings {
    /// All options at the `level` preset; options outside presets get their defaults.
    pub fn preset(level: Quality) -> Self {
        use AmbientOcclusion as Ao;
        use EffectLevel as Fx;
        use TextureFiltering as Tf;
        let optional = OptionalQuality::from(level);
        Self {
            view_distance: level,
            resolution_scale: ResolutionScale::default(),
            anti_aliasing: by_level(
                level,
                [
                    AntiAliasing::Fxaa,
                    AntiAliasing::Smaa,
                    AntiAliasing::Taa,
                    AntiAliasing::Taa,
                ],
            ),
            upscaling_method: UpscalingMethod::Off,
            upscaling_quality: UpscalingQuality::default(),
            image_sharpening: 50,
            dynamic_resolution: false,
            dynamic_resolution_target: DynamicResolutionTarget::default(),
            texture_quality: level,
            material_quality: level,
            texture_filtering: by_level(
                level,
                [
                    Tf::Trilinear,
                    Tf::Anisotropic4x,
                    Tf::Anisotropic8x,
                    Tf::Anisotropic16x,
                ],
            ),
            terrain_quality: level,
            foliage_quality: level,
            geometry_quality: level,
            object_detail: level,
            vegetation_density: level,
            shadow_quality: optional,
            shadow_distance: by_level(
                level,
                [
                    ShadowDistance::Near,
                    ShadowDistance::Medium,
                    ShadowDistance::Far,
                    ShadowDistance::Far,
                ],
            ),
            global_illumination: optional,
            ambient_occlusion: by_level(level, [Ao::Off, Ao::Ssao, Ao::Ssao, Ao::Gtao]),
            contact_shadows: by_level(level, [false, false, true, true]),
            reflection_quality: optional,
            water_quality: level,
            water_reflections: by_level(level, [false, true, true, true]),
            fog_quality: by_level(level, [Fx::Low, Fx::Medium, Fx::High, Fx::High]),
            volumetric_fog: by_level(level, [Fx::Off, Fx::Off, Fx::Medium, Fx::High]),
            light_shafts: by_level(level, [false, false, true, true]),
            weather_effects_quality: level,
            effects_quality: level,
            particle_quality: level,
            spell_effects_quality: level,
            particle_density: level,
            post_processing_quality: level,
            bloom: by_level(level, [Fx::Low, Fx::Medium, Fx::High, Fx::High]),
            depth_of_field: true,
            color_grading: true,
            vignette: true,
            // Off by default so the map and textures stay crisp.
            motion_blur: false,
            film_grain: false,
            chromatic_aberration: false,
        }
    }

    /// The preset every preset-controlled option matches; `None` means "Custom".
    pub fn overall(&self) -> Option<Quality> {
        Quality::ALL
            .iter()
            .copied()
            .find(|&level| self.preset_matches(level) == PRESET_OPTION_COUNT)
    }

    /// Sets every preset-controlled option to `level`; preferences stay as they are.
    pub fn apply_preset(&mut self, level: Quality) {
        let preset = Self::preset(level);
        for option in GraphicsOption::ALL {
            if option.in_preset() {
                option.select(self, option.index(&preset));
            }
        }
    }

    /// The preset with the most matching options (used to step out of "Custom").
    pub fn closest_preset(&self) -> Quality {
        Quality::ALL
            .iter()
            .copied()
            .max_by_key(|&level| self.preset_matches(level))
            .unwrap_or_default()
    }

    fn preset_matches(&self, level: Quality) -> usize {
        let preset = Self::preset(level);
        GraphicsOption::ALL
            .iter()
            .filter(|option| option.in_preset() && option.index(self) == option.index(&preset))
            .count()
    }

    pub(super) fn sanitized(mut self) -> Self {
        self.resolution_scale = self.resolution_scale.snapped();
        self.image_sharpening = self.image_sharpening.min(MAX_SHARPENING);
        if !UpscalingMethod::SUPPORTED.contains(&self.upscaling_method) {
            self.upscaling_method = UpscalingMethod::Off;
        }
        self
    }
}

/// Graphics options shown as selectors, in menu order. (Image Sharpening is a slider.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphicsOption {
    ViewDistance,
    ResolutionScale,
    AntiAliasing,
    UpscalingMethod,
    UpscalingQuality,
    DynamicResolution,
    DynamicResolutionTarget,
    TextureQuality,
    MaterialQuality,
    TextureFiltering,
    TerrainQuality,
    FoliageQuality,
    GeometryQuality,
    ObjectDetail,
    VegetationDensity,
    ShadowQuality,
    ShadowDistance,
    GlobalIllumination,
    AmbientOcclusion,
    ContactShadows,
    ReflectionQuality,
    WaterQuality,
    WaterReflections,
    FogQuality,
    VolumetricFog,
    LightShafts,
    WeatherEffectsQuality,
    EffectsQuality,
    ParticleQuality,
    SpellEffectsQuality,
    ParticleDensity,
    PostProcessingQuality,
    Bloom,
    DepthOfField,
    MotionBlur,
    ColorGrading,
    Vignette,
    FilmGrain,
    ChromaticAberration,
}

const PRESET_OPTION_COUNT: usize = {
    let mut count = 0;
    let mut i = 0;
    while i < GraphicsOption::ALL.len() {
        if GraphicsOption::ALL[i].in_preset() {
            count += 1;
        }
        i += 1;
    }
    count
};

/// Type-erased access to one option's value, so every selector row works the same way.
trait Slot {
    fn slot_count(&self) -> usize;
    fn slot_index(&self) -> usize;
    fn slot_label(&self) -> &'static str;
    fn slot_select(&mut self, index: usize);
}

impl<T: OptionValue> Slot for T {
    fn slot_count(&self) -> usize {
        T::ALL.len()
    }

    fn slot_index(&self) -> usize {
        self.index()
    }

    fn slot_label(&self) -> &'static str {
        self.label()
    }

    fn slot_select(&mut self, index: usize) {
        if let Some(&value) = T::ALL.get(index) {
            *self = value;
        }
    }
}

impl GraphicsOption {
    pub const ALL: [Self; 39] = [
        Self::ViewDistance,
        Self::ResolutionScale,
        Self::AntiAliasing,
        Self::UpscalingMethod,
        Self::UpscalingQuality,
        Self::DynamicResolution,
        Self::DynamicResolutionTarget,
        Self::TextureQuality,
        Self::MaterialQuality,
        Self::TextureFiltering,
        Self::TerrainQuality,
        Self::FoliageQuality,
        Self::GeometryQuality,
        Self::ObjectDetail,
        Self::VegetationDensity,
        Self::ShadowQuality,
        Self::ShadowDistance,
        Self::GlobalIllumination,
        Self::AmbientOcclusion,
        Self::ContactShadows,
        Self::ReflectionQuality,
        Self::WaterQuality,
        Self::WaterReflections,
        Self::FogQuality,
        Self::VolumetricFog,
        Self::LightShafts,
        Self::WeatherEffectsQuality,
        Self::EffectsQuality,
        Self::ParticleQuality,
        Self::SpellEffectsQuality,
        Self::ParticleDensity,
        Self::PostProcessingQuality,
        Self::Bloom,
        Self::DepthOfField,
        Self::MotionBlur,
        Self::ColorGrading,
        Self::Vignette,
        Self::FilmGrain,
        Self::ChromaticAberration,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::ViewDistance => "View Distance",
            Self::ResolutionScale => "Resolution Scale",
            Self::AntiAliasing => "Anti-Aliasing",
            Self::UpscalingMethod => "Upscaling Method",
            Self::UpscalingQuality => "Upscaling Quality",
            Self::DynamicResolution => "Dynamic Resolution",
            Self::DynamicResolutionTarget => "Dynamic Resolution Target",
            Self::TextureQuality => "Texture Quality",
            Self::MaterialQuality => "Material Quality",
            Self::TextureFiltering => "Texture Filtering",
            Self::TerrainQuality => "Terrain Quality",
            Self::FoliageQuality => "Foliage Quality",
            Self::GeometryQuality => "Geometry Quality",
            Self::ObjectDetail => "Object Detail",
            Self::VegetationDensity => "Vegetation Density",
            Self::ShadowQuality => "Shadow Quality",
            Self::ShadowDistance => "Shadow Distance",
            Self::GlobalIllumination => "Global Illumination",
            Self::AmbientOcclusion => "Ambient Occlusion",
            Self::ContactShadows => "Contact Shadows",
            Self::ReflectionQuality => "Reflection Quality",
            Self::WaterQuality => "Water Quality",
            Self::WaterReflections => "Water Reflections",
            Self::FogQuality => "Fog Quality",
            Self::VolumetricFog => "Volumetric Fog",
            Self::LightShafts => "Light Shafts",
            Self::WeatherEffectsQuality => "Weather Effects Quality",
            Self::EffectsQuality => "Effects Quality",
            Self::ParticleQuality => "Particle Quality",
            Self::SpellEffectsQuality => "Spell Effects Quality",
            Self::ParticleDensity => "Particle Density",
            Self::PostProcessingQuality => "Post Processing Quality",
            Self::Bloom => "Bloom",
            Self::DepthOfField => "Depth of Field",
            Self::MotionBlur => "Motion Blur",
            Self::ColorGrading => "Color Grading",
            Self::Vignette => "Vignette",
            Self::FilmGrain => "Film Grain",
            Self::ChromaticAberration => "Chromatic Aberration",
        }
    }

    /// Whether Overall Quality sets this option (see the module docs).
    pub const fn in_preset(self) -> bool {
        !matches!(
            self,
            Self::ResolutionScale
                | Self::UpscalingMethod
                | Self::UpscalingQuality
                | Self::DynamicResolution
                | Self::DynamicResolutionTarget
                | Self::DepthOfField
                | Self::MotionBlur
                | Self::ColorGrading
                | Self::Vignette
                | Self::FilmGrain
                | Self::ChromaticAberration
        )
    }

    /// Whether the row is shown at all: Upscaling Quality only with an upscaler.
    pub fn is_visible(self, g: &GraphicsSettings) -> bool {
        self != Self::UpscalingQuality || g.upscaling_method != UpscalingMethod::Off
    }

    /// Whether the value can be changed now. The manual Resolution Scale is locked
    /// while an upscaler or Dynamic Resolution controls the scale; the upscaler list is
    /// locked while the engine supports none.
    pub fn is_active(self, g: &GraphicsSettings) -> bool {
        match self {
            Self::ResolutionScale => {
                !g.dynamic_resolution && g.upscaling_method == UpscalingMethod::Off
            }
            Self::UpscalingMethod => UpscalingMethod::SUPPORTED.len() > 1,
            Self::DynamicResolutionTarget => g.dynamic_resolution,
            _ => true,
        }
    }

    fn slot(self, g: &mut GraphicsSettings) -> &mut dyn Slot {
        match self {
            Self::ViewDistance => &mut g.view_distance,
            Self::ResolutionScale => &mut g.resolution_scale,
            Self::AntiAliasing => &mut g.anti_aliasing,
            Self::UpscalingMethod => &mut g.upscaling_method,
            Self::UpscalingQuality => &mut g.upscaling_quality,
            Self::DynamicResolution => &mut g.dynamic_resolution,
            Self::DynamicResolutionTarget => &mut g.dynamic_resolution_target,
            Self::TextureQuality => &mut g.texture_quality,
            Self::MaterialQuality => &mut g.material_quality,
            Self::TextureFiltering => &mut g.texture_filtering,
            Self::TerrainQuality => &mut g.terrain_quality,
            Self::FoliageQuality => &mut g.foliage_quality,
            Self::GeometryQuality => &mut g.geometry_quality,
            Self::ObjectDetail => &mut g.object_detail,
            Self::VegetationDensity => &mut g.vegetation_density,
            Self::ShadowQuality => &mut g.shadow_quality,
            Self::ShadowDistance => &mut g.shadow_distance,
            Self::GlobalIllumination => &mut g.global_illumination,
            Self::AmbientOcclusion => &mut g.ambient_occlusion,
            Self::ContactShadows => &mut g.contact_shadows,
            Self::ReflectionQuality => &mut g.reflection_quality,
            Self::WaterQuality => &mut g.water_quality,
            Self::WaterReflections => &mut g.water_reflections,
            Self::FogQuality => &mut g.fog_quality,
            Self::VolumetricFog => &mut g.volumetric_fog,
            Self::LightShafts => &mut g.light_shafts,
            Self::WeatherEffectsQuality => &mut g.weather_effects_quality,
            Self::EffectsQuality => &mut g.effects_quality,
            Self::ParticleQuality => &mut g.particle_quality,
            Self::SpellEffectsQuality => &mut g.spell_effects_quality,
            Self::ParticleDensity => &mut g.particle_density,
            Self::PostProcessingQuality => &mut g.post_processing_quality,
            Self::Bloom => &mut g.bloom,
            Self::DepthOfField => &mut g.depth_of_field,
            Self::MotionBlur => &mut g.motion_blur,
            Self::ColorGrading => &mut g.color_grading,
            Self::Vignette => &mut g.vignette,
            Self::FilmGrain => &mut g.film_grain,
            Self::ChromaticAberration => &mut g.chromatic_aberration,
        }
    }

    /// Number of values.
    pub fn count(self) -> usize {
        let mut g = GraphicsSettings::default();
        self.slot(&mut g).slot_count()
    }

    /// Index of the current value.
    pub fn index(self, g: &GraphicsSettings) -> usize {
        let mut copy = *g;
        self.slot(&mut copy).slot_index()
    }

    pub fn value_label(self, g: &GraphicsSettings) -> &'static str {
        let mut copy = *g;
        self.slot(&mut copy).slot_label()
    }

    /// Sets the value at `index`; out-of-range indices are ignored.
    pub fn select(self, g: &mut GraphicsSettings, index: usize) {
        self.slot(g).slot_select(index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_the_high_preset() {
        let graphics = GraphicsSettings::default();
        assert_eq!(graphics.overall(), Some(Quality::High));
        assert!(!graphics.motion_blur && !graphics.film_grain && !graphics.chromatic_aberration);
        assert_eq!(graphics.resolution_scale, ResolutionScale(100));
        assert_eq!(graphics.upscaling_method, UpscalingMethod::Off);
    }

    #[test]
    fn every_preset_is_recognized_and_custom_otherwise() {
        for &level in Quality::ALL {
            assert_eq!(GraphicsSettings::preset(level).overall(), Some(level));
        }
        let graphics = GraphicsSettings {
            shadow_quality: OptionalQuality::Off,
            ..GraphicsSettings::default()
        };
        assert_eq!(graphics.overall(), None);
        assert_eq!(graphics.closest_preset(), Quality::High);
    }

    #[test]
    fn presets_keep_personal_preferences() {
        let mut graphics = GraphicsSettings {
            motion_blur: true,
            resolution_scale: ResolutionScale(70),
            dynamic_resolution: true,
            ..GraphicsSettings::default()
        };
        graphics.apply_preset(Quality::Low);
        assert_eq!(graphics.overall(), Some(Quality::Low));
        assert_eq!(graphics.anti_aliasing, AntiAliasing::Fxaa);
        assert!(graphics.motion_blur, "preference kept");
        assert_eq!(graphics.resolution_scale, ResolutionScale(70));
        assert!(graphics.dynamic_resolution);
        // Changing a preference does not make the preset "Custom".
        graphics.film_grain = true;
        assert_eq!(graphics.overall(), Some(Quality::Low));
    }

    #[test]
    fn options_read_and_write_through_indices() {
        let mut graphics = GraphicsSettings::default();
        let filtering = GraphicsOption::TextureFiltering;
        assert_eq!(filtering.count(), 6);
        assert_eq!(filtering.value_label(&graphics), "Anisotropic 8×");
        filtering.select(&mut graphics, 5);
        assert_eq!(graphics.texture_filtering, TextureFiltering::Anisotropic16x);
        filtering.select(&mut graphics, 99);
        assert_eq!(graphics.texture_filtering, TextureFiltering::Anisotropic16x);

        let contact = GraphicsOption::ContactShadows;
        assert_eq!(contact.value_label(&graphics), "On");
        contact.select(&mut graphics, 0);
        assert!(!graphics.contact_shadows);
        assert_eq!(GraphicsOption::Bloom.count(), 4);
        assert_eq!(GraphicsOption::ShadowQuality.count(), 5);
    }

    #[test]
    fn locked_and_hidden_options_follow_the_rules() {
        let mut graphics = GraphicsSettings::default();
        assert!(GraphicsOption::ResolutionScale.is_active(&graphics));
        assert!(!GraphicsOption::DynamicResolutionTarget.is_active(&graphics));
        assert!(
            !GraphicsOption::UpscalingMethod.is_active(&graphics),
            "no upscaler yet"
        );
        assert!(!GraphicsOption::UpscalingQuality.is_visible(&graphics));
        graphics.dynamic_resolution = true;
        assert!(!GraphicsOption::ResolutionScale.is_active(&graphics));
        assert!(GraphicsOption::DynamicResolutionTarget.is_active(&graphics));
    }

    #[test]
    fn sanitizing_snaps_scale_and_drops_unsupported_upscalers() {
        let graphics = GraphicsSettings {
            resolution_scale: ResolutionScale(73),
            image_sharpening: 250,
            upscaling_method: UpscalingMethod::Dlss,
            ..GraphicsSettings::default()
        }
        .sanitized();
        assert_eq!(graphics.resolution_scale, ResolutionScale(70));
        assert_eq!(graphics.image_sharpening, MAX_SHARPENING);
        assert_eq!(graphics.upscaling_method, UpscalingMethod::Off);
    }

    #[test]
    fn preset_option_count_matches_the_flags() {
        let counted = GraphicsOption::ALL.iter().filter(|o| o.in_preset()).count();
        assert_eq!(PRESET_OPTION_COUNT, counted);
        assert_eq!(counted, 28);
    }
}
