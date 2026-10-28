//! Standard rendering from frozen numeric catalogs. Importer-owned names never enter mesh loops.
use crate::{
    RenderSource,
    assets::VoxelAssets,
    fog::FogLut,
    material::{ModelBlend, ModelMaterial, VoxelCutoutMaterial, VoxelMaterial, WaterMaterial},
    models::{CellInfo, ModelQuad, cell},
};
use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::{
        render_resource::{Extent3d, TextureDimension, TextureFormat, TextureViewDimension},
        storage::ShaderBuffer,
    },
};
use hollow_core::{BlockId, FluidId, ResourceId, StateId, content::ContentCatalog};
use hollow_engine::fluids::FluidCatalog;
use std::{collections::HashMap, io, sync::Arc};

#[derive(Resource, Clone)]
pub struct CatalogRenderSource(Arc<RenderData>);
struct RenderData {
    content: ContentCatalog,
    fluids: FluidCatalog,
    cells: Box<[CellInfo]>,
    layers: Box<[Option<u16>]>,
    opaque: Box<[bool]>,
    entity_blocks: Box<[bool]>,
}
/// Only explicitly registered visual resources are interpreted as terrain textures.
/// The caller owns image loading; this preparation step runs once before streaming.
pub struct RenderTextures<I: ExactSizeIterator<Item = io::Result<(ResourceId, Image)>>>(pub I);
#[derive(bevy::ecs::system::SystemParam)]
pub struct RenderAssetStores<'w> {
    pub images: ResMut<'w, Assets<Image>>,
    pub buffers: ResMut<'w, Assets<ShaderBuffer>>,
    pub terrain: ResMut<'w, Assets<VoxelMaterial>>,
    pub cutout: ResMut<'w, Assets<VoxelCutoutMaterial>>,
    pub models: ResMut<'w, Assets<ModelMaterial>>,
}
impl CatalogRenderSource {
    pub fn prepare(
        content: ContentCatalog,
        fluids: FluidCatalog,
        textures: RenderTextures<impl ExactSizeIterator<Item = io::Result<(ResourceId, Image)>>>,
        stores: &mut RenderAssetStores,
        fog: &FogLut,
    ) -> io::Result<(Self, VoxelAssets)> {
        let count = textures.0.len();
        let mut textures = textures.0.peekable();
        if count == 0 || count > 2048 {
            return Err(io::Error::other(
                "Voxel texture array requires 1..=2048 layers",
            ));
        }
        let size = match textures.peek().expect("Nonempty textures") {
            Ok((_, image)) => image.texture_descriptor.size,
            Err(error) => return Err(io::Error::other(error.to_string())),
        };
        let bytes_per_layer = (size.width as usize)
            .checked_mul(size.height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .filter(|bytes| *bytes > 0)
            .ok_or_else(|| io::Error::other("Invalid texture dimensions"))?;
        let capacity = bytes_per_layer
            .checked_mul(count)
            .ok_or_else(|| io::Error::other("Voxel texture array size overflow"))?;
        let mut layers = vec![None; content.resource_count()];
        let mut opaque = Vec::with_capacity(count);
        let mut pixels = Vec::with_capacity(capacity);
        for (index, image) in textures.enumerate() {
            let (resource, image) = image?;
            let descriptor = &image.texture_descriptor;
            if descriptor.size != size
                || size.depth_or_array_layers != 1
                || descriptor.format != TextureFormat::Rgba8UnormSrgb
                || descriptor.mip_level_count != 1
                || descriptor.dimension != TextureDimension::D2
                || descriptor.sample_count != 1
                || resource.index() >= layers.len()
                || layers[resource.index()].is_some()
            {
                return Err(io::Error::other(
                    "Voxel textures must be distinct, equally sized RGBA8 sRGB images",
                ));
            }
            let data = image
                .data
                .ok_or_else(|| io::Error::other("Voxel texture has no CPU pixels"))?;
            if data.len() != bytes_per_layer {
                return Err(io::Error::other("Invalid voxel texture pixel count"));
            }
            opaque.push(data.chunks_exact(4).all(|pixel| pixel[3] == 255));
            pixels.extend_from_slice(&data);
            layers[resource.index()] = Some(index as u16);
        }
        let count = opaque.len();
        let mut image = Image::new(
            Extent3d {
                depth_or_array_layers: count as u32,
                ..size
            },
            TextureDimension::D2,
            pixels,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        image.texture_view_descriptor =
            Some(bevy::render::render_resource::TextureViewDescriptor {
                dimension: Some(TextureViewDimension::D2Array),
                ..default()
            });
        image.sampler = bevy::image::ImageSampler::nearest();
        let blocks = stores.images.add(image);
        let layer = |resource: ResourceId| -> io::Result<u16> {
            layers
                .get(resource.index())
                .copied()
                .flatten()
                .ok_or_else(|| io::Error::other(format!("Missing visual resource: {}", resource.0)))
        };
        let mut cells = vec![CellInfo::default(); content.state_count()];
        let mut faces = vec![[1.0, 1.0, 1.0, 0.0]; 6];
        // This cold index deduplicates appearances before the 14-bit GPU domain fills.
        // It is discarded after preparation; runtime cells address the packed table directly.
        let mut appearances = HashMap::<[u16; 6], u16>::new();
        for raw in 0..content.state_count() {
            let definition = content.state(StateId(raw as u32));
            let mut flags = 0;
            if definition.air {
                flags |= cell::AIR;
            }
            if definition.full_cube && definition.opacity == 15 {
                flags |= cell::OCCLUDES;
            }
            if definition.full_cube {
                flags |= cell::FLUID_OCCLUDES;
            }
            if definition.occludes
                && !definition.full_cube
                && !content.shapes(definition.selection).is_empty()
            {
                flags |= cell::FLUID_OCCLUDER;
            }
            let mut appearance = 0;
            if let Some(visual) = content.visual(definition.block) {
                let mut slots = [0; 6];
                for (slot, resource) in slots.iter_mut().zip(visual.faces) {
                    *slot = layer(resource)?;
                }
                if definition.full_cube && !visual.translucent {
                    appearance = if let Some(&appearance) = appearances.get(&slots) {
                        appearance
                    } else {
                        let id = faces.len() / 6;
                        if id >= 1 << 14 {
                            return Err(io::Error::other(
                                "Voxel appearances exceed 14-bit vertex storage",
                            ));
                        }
                        faces.extend(slots.map(|slot| [1.0, 1.0, 1.0, slot as f32]));
                        appearances.insert(slots, id as u16);
                        id as u16
                    };
                    flags |= if slots.iter().all(|&slot| opaque[slot as usize]) {
                        cell::CUBE
                    } else {
                        cell::CUTOUT
                    };
                } else {
                    flags |= cell::MODEL;
                }
            }
            let fluid = definition.fluid;
            let visible = fluid.id != FluidId::NONE && fluids.profile(fluid.id).render.is_some();
            if visible {
                let render = fluids.profile(fluid.id).render.unwrap();
                layer(render.still)?;
                layer(render.flowing)?;
                if fluid.id.0 < 1 << 14 && render.ocean_surface && render.translucent {
                    flags |= cell::OCEAN_SURFACE;
                }
            }
            cells[raw] = CellInfo {
                appearance,
                flags,
                fluid: if visible {
                    u32::from(fluid.id.0) << 8
                        | u32::from(fluid.amount)
                        | u32::from(fluid.falling) << 24
                } else {
                    0
                },
            };
        }
        let faces = stores
            .buffers
            .add(ShaderBuffer::new(faces, RenderAssetUsages::RENDER_WORLD));
        let mut model_faces: Vec<_> = (0..count)
            .map(|layer| [1.0, 1.0, 1.0, layer as f32])
            .collect();
        model_faces.extend([[1.0, 1.0, 1.0, 0.0]; 8]);
        let model_faces = stores.buffers.add(ShaderBuffer::new(
            model_faces,
            RenderAssetUsages::RENDER_WORLD,
        ));
        let animations = stores.buffers.add(ShaderBuffer::new(
            vec![[0u32; 4]],
            RenderAssetUsages::RENDER_WORLD,
        ));
        let water = WaterMaterial::new(None, 0, 0.0, false, fog);
        let models = [
            ModelBlend::Opaque,
            ModelBlend::Cutout,
            ModelBlend::Translucent,
        ]
        .map(|blend| {
            stores.models.add(ModelMaterial::new(
                blocks.clone(),
                model_faces.clone(),
                &water,
                blend,
                None,
                animations.clone(),
            ))
        });
        let [models, models_cutout, translucent] = models;
        let cutout = stores.cutout.add(VoxelCutoutMaterial {
            blocks: blocks.clone(),
            faces: faces.clone(),
            animations: animations.clone(),
            fog: fog.0.clone(),
            fog_sky: fog.1.clone(),
            fog_aerial: fog.2.clone(),
        });
        let terrain = stores.terrain.add(VoxelMaterial {
            blocks,
            faces,
            animations,
            fog: fog.0.clone(),
            fog_sky: fog.1.clone(),
            fog_aerial: fog.2.clone(),
        });
        let entity_blocks = vec![false; content.block_count()].into_boxed_slice();
        Ok((
            Self(Arc::new(RenderData {
                entity_blocks,
                content,
                fluids,
                cells: cells.into(),
                layers: layers.into(),
                opaque: opaque.into(),
            })),
            VoxelAssets {
                terrain,
                cutout,
                models,
                models_cutout,
                translucent,
            },
        ))
    }
    /// Configure entity-rendered blocks before sharing this source with mesh jobs.
    pub fn render_as_entity(&mut self, block: BlockId) {
        let data =
            Arc::get_mut(&mut self.0).expect("Configure visuals before sharing the render source");
        data.entity_blocks[block.index()] = true;
        let definition = data.content.block(block);
        for cell in
            &mut data.cells[definition.states.start as usize..definition.states.end as usize]
        {
            cell.flags = (cell.flags & !(cell::CUBE | cell::CUTOUT)) | cell::MODEL;
        }
    }
    pub fn texture_slot(&self, resource: ResourceId) -> Option<u32> {
        self.0
            .layers
            .get(resource.index())
            .copied()
            .flatten()
            .map(u32::from)
    }
}
impl RenderSource for CatalogRenderSource {
    type Cell = StateId;
    fn content(&self) -> &ContentCatalog {
        &self.0.content
    }
    fn fluids(&self) -> &FluidCatalog {
        &self.0.fluids
    }
    fn cell_info(&self, state: StateId) -> CellInfo {
        self.0.cells[state.index()]
    }
    fn visit_quads(&self, state: StateId, _world: IVec3, mut visit: impl FnMut(&ModelQuad)) {
        let definition = self.content().state(state);
        let Some(visual) = self.content().visual(definition.block) else {
            return;
        };
        for bounds in self.content().shapes(definition.selection) {
            for face in bounds.faces() {
                visit(&ModelQuad {
                    positions: face.positions.map(|position| position.to_array()),
                    normal: face.normal.to_array(),
                    uv: [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
                    texture: self
                        .texture_slot(visual.faces[face.axis * 2 + usize::from(face.positive)])
                        .expect("Prepared visual"),
                    tinted: false,
                    cull: if definition.full_cube {
                        face.normal.as_ivec3().to_array()
                    } else {
                        [0; 3]
                    },
                    translucent: visual.translucent,
                    effect: 0,
                    light_emission: definition.luminance,
                    shade_up: false,
                });
            }
        }
    }
    fn fluid_texture(&self, fluid: FluidId, flowing: bool) -> u32 {
        let render = self.fluids().profile(fluid).render.expect("Visible fluid");
        self.texture_slot(if flowing {
            render.flowing
        } else {
            render.still
        })
        .expect("Prepared fluid texture")
    }
    fn entity_rendered(&self, state: StateId) -> bool {
        self.0.entity_blocks[self.content().state(state).block.index()]
    }
    fn tint_index(&self, _block: BlockId) -> u8 {
        0
    }
    fn opaque_texture(&self, slot: u32) -> bool {
        self.0.opaque.get(slot as usize).copied().unwrap_or(false)
    }
}
