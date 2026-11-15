//! Reusable sky surfaces and geometry; consumers supply time, palettes and patterns.
mod clouds;
use crate::environment::{SkyMaterial, SkyPass, SkySettings};
use bevy::{
    asset::RenderAssetUsages,
    camera::visibility::NoFrustumCulling,
    ecs::system::SystemParam,
    light::NotShadowCaster,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};
pub use clouds::{CloudFrame, CloudGeometry, CloudLayer, CloudPattern, update_clouds};

#[derive(Component)]
#[require(Transform, NotShadowCaster)]
pub struct SkySurface;

#[derive(SystemParam)]
pub struct SkyAssets<'w> {
    meshes: ResMut<'w, Assets<Mesh>>,
    materials: ResMut<'w, Assets<SkyMaterial>>,
    palette: Res<'w, crate::fog::SkyPalette>,
}
impl SkyAssets<'_> {
    pub fn surface(&mut self, mesh: Mesh, pass: SkyPass) -> impl Bundle + use<> {
        (
            NoFrustumCulling,
            Mesh3d(self.meshes.add(mesh)),
            self.material(pass),
        )
    }
    pub fn material(&mut self, pass: SkyPass) -> impl Bundle + use<> {
        (
            SkySurface,
            MeshMaterial3d(self.materials.add(SkyMaterial {
                pass,
                settings: SkySettings::default(),
                palette: self.palette.0.clone(),
            })),
        )
    }
}

/// A depth-tested background reads the same palette as terrain fog and water.
#[derive(Component)]
pub struct SkyBackground;

pub fn update_background_bounds(
    camera: Single<&GlobalTransform, With<crate::VoxelViewer>>,
    mut backgrounds: Query<&mut bevy::camera::primitives::Aabb, With<SkyBackground>>,
) {
    let center = camera.translation().into();
    for mut bounds in backgrounds
        .contiguous_iter_mut()
        .expect("Dense sky bounds query")
    {
        // The vertex shader covers the whole view. Keeping the camera inside its
        // bounds prevents GPU occlusion from treating it as a quad at the origin.
        for index in 0..bounds.len() {
            hollow_features::query::set_field_if_neq(
                &mut bounds,
                index,
                |bounds| &mut bounds.center,
                center,
            );
        }
    }
}

pub fn background_mesh() -> Mesh {
    // A single triangle avoids redundant shading along a screen quad's diagonal.
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[-1.0, -1.0, 0.0], [3.0, -1.0, 0.0], [-1.0, 3.0, 0.0]],
    )
}

pub struct Star {
    pub center: Vec3,
    pub size: f32,
    pub color: [f32; 4],
}
pub fn star_mesh(stars: impl ExactSizeIterator<Item = Star>) -> Mesh {
    let mut positions = Vec::with_capacity(stars.len() * 4);
    let mut uvs = Vec::with_capacity(stars.len() * 4);
    let mut colors = Vec::with_capacity(stars.len() * 4);
    let mut indices = Vec::with_capacity(stars.len() * 6);
    for (
        index,
        Star {
            center,
            size,
            color,
        },
    ) in stars.enumerate()
    {
        let (tangent, bitangent) = center.any_orthonormal_pair();
        for (u, v) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            positions.push((center + (tangent * u + bitangent * v) * size).to_array());
            uvs.push([u, v]);
            colors.push(color);
        }
        let base = u32::try_from(index * 4).expect("Star vertex index fits u32");
        indices.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U32(indices))
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SkyPresentation;
pub struct SkyPresentationPlugin;
impl Plugin for SkyPresentationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CloudFrame>().add_systems(
            PostUpdate,
            update_background_bounds
                .after(bevy::transform::TransformSystems::Propagate)
                .before(bevy::camera::visibility::VisibilitySystems::CheckVisibility),
        );
        app.add_systems(
            Update,
            update_clouds
                .in_set(SkyPresentation)
                .run_if(resource_exists::<CloudGeometry>)
                .run_if(resource_exists::<CloudPattern>),
        );
    }
}
