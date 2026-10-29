use crate::voxel::TexturePackId;
use bevy::{
    prelude::*,
    settings::{ReflectSettingsGroup, SettingsGroup},
};

pub(super) const FOV_MIN: f32 = 60.0;
pub(super) const FOV_MAX: f32 = 110.0;
pub(super) const VIEW_DISTANCE_MIN: u32 = 4;
pub(super) const VIEW_DISTANCE_MAX: u32 = 32;

#[derive(Component, Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[type_path = "hollow_minecraft::settings"]
pub enum AntiAliasingMode {
    Off,
    Fxaa,
    Msaa2x,
    #[default]
    Msaa4x,
    Msaa8x,
}

impl AntiAliasingMode {
    pub(super) const fn msaa(self) -> Msaa {
        match self {
            Self::Off | Self::Fxaa => Msaa::Off,
            Self::Msaa2x => Msaa::Sample2,
            Self::Msaa4x => Msaa::Sample4,
            Self::Msaa8x => Msaa::Sample8,
        }
    }
}

#[derive(Component, Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[type_path = "hollow_minecraft::settings"]
pub enum ShadowQuality {
    Off,
    Low,
    #[default]
    Medium,
    High,
}

impl ShadowQuality {
    pub(super) const fn map_size(self) -> Option<usize> {
        match self {
            Self::Off => None,
            Self::Low => Some(1024),
            Self::Medium => Some(2048),
            Self::High => Some(4096),
        }
    }
}

#[derive(Component, Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[type_path = "hollow_minecraft::settings"]
pub enum DisplayMode {
    Windowed,
    #[default]
    BorderlessFullscreen,
}

#[derive(Component, Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[type_path = "hollow_minecraft::settings"]
pub enum RenderResolution {
    #[default]
    Native,
    R480p,
    R720p,
    R900p,
    R1080p,
    R1440p,
    R2160p,
}

impl RenderResolution {
    /// Keep the window's aspect ratio, including ultrawide and portrait displays.
    pub(crate) fn size(self, window: UVec2, max_dimension: u32) -> UVec2 {
        let window = window.max(UVec2::ONE);
        let height = match self {
            Self::Native => return window,
            Self::R480p => 480,
            Self::R720p => 720,
            Self::R900p => 900,
            Self::R1080p => 1080,
            Self::R1440p => 1440,
            Self::R2160p => 2160,
        };
        let width = (u64::from(window.x) * u64::from(height) + u64::from(window.y) / 2)
            / u64::from(window.y);
        // Very wide, short windows can otherwise exceed the GPU's texture limit.
        let limit = u64::from(max_dimension.max(1));
        if width > limit {
            UVec2::new(
                limit as u32,
                (u64::from(height) * limit / width).max(1) as u32,
            )
        } else {
            UVec2::new(width.max(1) as u32, height)
        }
    }
}

#[derive(Resource, SettingsGroup, Reflect, Clone, Copy, Debug, PartialEq)]
#[component(immutable)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "graphics")]
#[type_path = "hollow_minecraft::settings"]
pub struct GraphicsSettings {
    pub anti_aliasing: AntiAliasingMode,
    pub vsync: bool,
    /// GPU occlusion culling (experimental in Bevy): skips terrain hidden behind terrain.
    pub occlusion_culling: bool,
    pub shadow_quality: ShadowQuality,
    /// Glow around the sun, the moon and bright light sources.
    pub bloom: bool,
    /// Vanilla-style cloud layer (`crate::sky`).
    pub clouds: bool,
    pub damage_camera_motion: bool,
    pub gameplay_volume: f32,
    pub texture_pack: TexturePackId,
    pub field_of_view: f32,
    pub view_distance: u32,
    pub display_mode: DisplayMode,
    pub render_resolution: RenderResolution,
}

impl Default for GraphicsSettings {
    fn default() -> Self {
        Self {
            anti_aliasing: AntiAliasingMode::Msaa4x,
            vsync: true,
            occlusion_culling: true,
            shadow_quality: ShadowQuality::Medium,
            bloom: true,
            clouds: true,
            damage_camera_motion: true,
            gameplay_volume: 60.0,
            texture_pack: TexturePackId::default(),
            field_of_view: 85.0,
            view_distance: 14,
            display_mode: DisplayMode::BorderlessFullscreen,
            render_resolution: RenderResolution::Native,
        }
    }
}

impl GraphicsSettings {
    pub(super) fn normalized(mut self) -> Self {
        self.field_of_view = normalized_fov(self.field_of_view);
        self.view_distance = self.effective_view_distance();
        self.gameplay_volume = if self.gameplay_volume.is_finite() {
            self.gameplay_volume.clamp(0.0, 100.0)
        } else {
            60.0
        };
        if !self.texture_pack.is_available() {
            self.texture_pack = TexturePackId::Faithful;
        }
        self
    }

    pub(crate) fn effective_view_distance(&self) -> u32 {
        self.view_distance
            .clamp(VIEW_DISTANCE_MIN, VIEW_DISTANCE_MAX)
    }
}

pub(super) fn normalized_fov(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(FOV_MIN, FOV_MAX)
    } else {
        GraphicsSettings::default().field_of_view
    }
}

/// Gameplay distance is independent of the camera and graphics settings.
#[derive(Resource, SettingsGroup, Reflect, Clone, Copy, Debug, PartialEq, Eq)]
#[component(immutable)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "simulation")]
#[type_path = "hollow_minecraft::settings"]
pub struct SimulationSettings {
    pub simulation_distance: u32,
}
impl Default for SimulationSettings {
    fn default() -> Self {
        Self {
            simulation_distance: 6,
        }
    }
}
impl SimulationSettings {
    pub(crate) fn effective_distance(&self) -> u32 {
        self.simulation_distance.clamp(2, 12)
    }
}
