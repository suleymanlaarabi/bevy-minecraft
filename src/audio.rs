//! Shared vanilla sound bank: registry-indexed block profiles and weighted variants.
use std::collections::HashMap;

use bevy::{
    audio::{SpatialScale, Volume},
    ecs::storage::SparseSet,
    prelude::*,
};
pub(crate) use pumpkin_data::sound::Sound as SoundId;
use serde::Deserialize;

use crate::{
    character::{
        CharacterActivity, CharacterBody, CharacterMotion, CharacterMovement, CharacterState,
    },
    game::GameState,
    player::{Player, PlayerCamera},
    voxel::{VoxelKind, VoxelWorld},
};

pub struct GameAudioPlugin;

impl Plugin for GameAudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_bank)
            .add_observer(play_sound)
            .add_observer(button_audio)
            .add_systems(Update, equipment_audio.run_if(crate::world::playing))
            .add_systems(
                FixedUpdate,
                movement_audio
                    .after(crate::character::CharacterSystems)
                    .run_if(crate::world::playing)
                    .run_if(crate::survival::player_alive)
                    .run_if(not(resource_exists::<crate::character::SimulationPaused>)),
            );
    }
}

#[derive(Clone, Copy)]
pub(crate) enum BlockSound {
    Break,
    Step,
    Place,
    Hit,
    Fall,
    Open,
    Close,
}

#[derive(Clone, Copy)]
enum SoundKey {
    Event(SoundId),
    Block(VoxelKind, BlockSound),
}

#[derive(Event, Clone, Copy)]
pub(crate) struct Sound {
    key: SoundKey,
    position: Option<Vec3>,
    volume: f32,
    pitch: f32,
}

impl Sound {
    pub fn event(id: SoundId) -> Self {
        Self {
            key: SoundKey::Event(id),
            position: None,
            volume: if id == SoundId::EntityItemPickup {
                0.25
            } else {
                1.0
            },
            pitch: 1.0,
        }
    }

    pub fn at(id: SoundId, position: Vec3) -> Self {
        Self {
            position: Some(position),
            ..Self::event(id)
        }
    }

    pub fn block(kind: VoxelKind, action: BlockSound, position: IVec3) -> Self {
        let (volume, pitch) = match action {
            BlockSound::Step => (0.5, 1.0),
            BlockSound::Hit => (0.25, 0.5),
            BlockSound::Place => (1.0, 0.8),
            _ => (1.0, 1.0),
        };
        Self {
            key: SoundKey::Block(kind, action),
            position: Some(position.as_vec3() + Vec3::splat(0.5)),
            volume,
            pitch,
        }
    }
}

#[derive(Deserialize)]
struct ImportedBank {
    blocks: HashMap<String, ImportedBlock>,
    events: HashMap<String, Vec<ImportedVariant>>,
}

#[derive(Deserialize)]
struct ImportedBlock {
    volume: f32,
    pitch: f32,
    #[serde(flatten)]
    sounds: HashMap<String, String>,
}

#[derive(Deserialize)]
struct ImportedVariant {
    path: String,
    volume: f32,
    pitch: f32,
    weight: f32,
}

struct Variant {
    source: Handle<AudioSource>,
    volume: f32,
    pitch: f32,
    weight: f32,
}

struct SoundEvent {
    variants: Box<[Variant]>,
    weight: f32,
}

#[derive(Default)]
struct BlockProfile {
    events: [Option<SoundId>; 7],
    volume: f32,
    pitch: f32,
}

#[derive(Resource, Default)]
struct SoundBank {
    // Registry IDs index dense event storage without retaining imported names.
    events: SparseSet<u16, SoundEvent>,
    blocks: Box<[BlockProfile]>,
    random: u64,
}

impl SoundBank {
    fn random(&mut self) -> f32 {
        self.random ^= self.random << 13;
        self.random ^= self.random >> 7;
        self.random ^= self.random << 17;
        (self.random >> 40) as f32 / (1u32 << 24) as f32
    }
}

fn load_bank(mut commands: Commands, server: Res<AssetServer>) {
    let imported = std::fs::read(crate::asset_directory().join("audio/vanilla/sounds.json"))
        .and_then(|bytes| {
            serde_json::from_slice::<ImportedBank>(&bytes).map_err(std::io::Error::other)
        });
    let mut bank = SoundBank {
        random: 0x716a_9d38_4b25_c6ef,
        ..default()
    };
    match imported {
        Ok(imported) => {
            for (name, variants) in imported.events {
                let Some(id) = SoundId::from_name(&name) else {
                    warn!("Unknown imported sound event: {name}");
                    continue;
                };
                bank.events.insert(
                    id as u16,
                    SoundEvent {
                        weight: variants.iter().map(|variant| variant.weight).sum(),
                        variants: variants
                            .into_iter()
                            .map(|variant| Variant {
                                source: server.load(format!("audio/vanilla/{}", variant.path)),
                                volume: variant.volume,
                                pitch: variant.pitch,
                                weight: variant.weight,
                            })
                            .collect(),
                    },
                );
            }
            bank.blocks = VoxelKind::ALL
                .iter()
                .map(|kind| {
                    let Some(block) = imported.blocks.get(kind.name()) else {
                        warn!("Missing vanilla sound profile for {}", kind.name());
                        return BlockProfile::default();
                    };
                    BlockProfile {
                        events: ["break", "step", "place", "hit", "fall", "open", "close"].map(
                            |action| {
                                block
                                    .sounds
                                    .get(action)
                                    .and_then(|name| SoundId::from_name(name))
                            },
                        ),
                        volume: block.volume,
                        pitch: block.pitch,
                    }
                })
                .collect();
        }
        Err(error) => {
            warn!("Cannot load gameplay sounds: {error}. Run python scripts/import_sounds.py")
        }
    }
    commands.insert_resource(bank);
}

fn play_sound(
    sound: On<Sound>,
    mut commands: Commands,
    mut bank: ResMut<SoundBank>,
    settings: Res<crate::settings::GraphicsSettings>,
    listener: Query<&GlobalTransform, With<PlayerCamera>>,
) {
    if settings.gameplay_volume <= 0.0 {
        return;
    }
    if let Some(position) = sound.position {
        let Ok(listener) = listener.single() else {
            return;
        };
        if listener.translation().distance_squared(position) > 32.0 * 32.0 {
            return;
        }
    }
    let (event, volume, pitch) = match sound.key {
        SoundKey::Event(id) => (Some(id), sound.volume, sound.pitch),
        SoundKey::Block(kind, action) => {
            let Some(profile) = bank.blocks.get(kind.index()) else {
                return;
            };
            (
                profile.events[action as usize],
                sound.volume * profile.volume,
                sound.pitch * profile.pitch,
            )
        }
    };
    let Some(event) = event else { return };
    let choice = bank.random();
    let variation = 0.9 + bank.random() * 0.2;
    let Some(event) = bank.events.get(event as u16) else {
        return;
    };
    let mut remaining = choice * event.weight;
    let Some(variant) = event
        .variants
        .iter()
        .find(|variant| {
            remaining -= variant.weight;
            remaining < 0.0
        })
        .or_else(|| event.variants.last())
    else {
        return;
    };
    commands.spawn((
        AudioPlayer::new(variant.source.clone()),
        PlaybackSettings {
            volume: Volume::Linear(settings.gameplay_volume / 100.0 * volume * variant.volume),
            speed: pitch * variant.pitch * variation,
            spatial: sound.position.is_some(),
            spatial_scale: Some(SpatialScale::new(0.1)),
            ..PlaybackSettings::DESPAWN
        },
        Transform::from_translation(sound.position.unwrap_or_default()),
        DespawnOnExit(GameState::Game),
    ));
}

/// Per-player cadence survives render frames but resets across teleports and sessions.
#[derive(Component, Default)]
pub(crate) struct MovementAudio {
    previous: Option<(Vec3, bool, bool)>,
    distance: f32,
}

fn movement_audio(
    mut commands: Commands,
    world: VoxelWorld,
    mut players: Query<
        (
            &CharacterMotion,
            &CharacterBody,
            &CharacterState,
            &CharacterMovement,
            &CharacterActivity,
            &mut MovementAudio,
            Has<crate::character::Flying>,
        ),
        With<Player>,
    >,
) {
    for (motion, body, state, movement, activity, mut audio, flying) in &mut players {
        let previous =
            audio
                .previous
                .replace((motion.position.as_vec3(), state.on_ground, state.in_water));
        if !activity.simulated || flying || activity.distance > 4.0 {
            audio.distance = 0.0;
            continue;
        }
        let feet = motion.position.as_vec3() - Vec3::Y * body.origin_height;
        let Some((previous_position, grounded, water)) = previous else {
            continue;
        };
        if previous_position.distance_squared(motion.position.as_vec3()) > 16.0 {
            audio.distance = 0.0;
            continue;
        }
        if state.in_water && !water {
            commands.trigger(Sound::event(crate::audio::SoundId::EntityPlayerSplash));
        }
        // Fences and walls can support feet more than one block above their cell.
        let floor = (feet - Vec3::Y * 0.2).floor().as_ivec3();
        let support = [floor, floor - IVec3::Y].into_iter().find_map(|position| {
            world
                .state(position)
                .filter(|state| !state.native().is_air())
                .map(|state| (position, state))
        });
        if state.on_ground && !grounded && !state.in_water {
            if let Some((position, block)) = support {
                commands.trigger(Sound::block(block.kind(), BlockSound::Fall, position));
            }
        }
        let climbing = activity.climbing;
        if movement.sneak || (!state.on_ground && !state.in_water && !climbing) {
            audio.distance = 0.0;
            continue;
        }
        audio.distance += activity.distance
            + if climbing {
                activity.vertical.abs()
            } else {
                0.0
            };
        if audio.distance < if state.in_water { 0.8 } else { 1.0 / 0.6 } {
            continue;
        }
        audio.distance = 0.0;
        if state.in_water {
            commands.trigger(Sound::event(crate::audio::SoundId::EntityPlayerSwim));
        } else {
            let contact = if climbing {
                let position = feet.floor().as_ivec3();
                world.state(position).map(|state| (position, state))
            } else {
                // Snow layers and other non-solid coverings override the support sound.
                let position = (feet + Vec3::Y * 0.05).floor().as_ivec3();
                world
                    .state(position)
                    .filter(|state| !state.native().is_air())
                    .map(|state| (position, state))
                    .or(support)
            };
            if let Some((position, block)) = contact {
                commands.trigger(Sound::block(block.kind(), BlockSound::Step, position));
            }
        }
    }
}

fn button_audio(
    event: On<bevy::ui_widgets::Activate>,
    buttons: Query<(), With<bevy::feathers::controls::FeathersButton>>,
    mut commands: Commands,
) {
    if buttons.contains(event.entity) {
        let mut sound = Sound::event(crate::audio::SoundId::UiButtonClick);
        sound.volume = 0.25;
        commands.trigger(sound);
    }
}

#[derive(Component, Default)]
pub(crate) struct EquipmentAudio(Option<[Option<crate::inventory::ItemKind>; 4]>);

fn equipment_audio(
    mut commands: Commands,
    mut players: Query<
        (&crate::inventory::PlayerInventory, &mut EquipmentAudio),
        (With<Player>, Changed<crate::inventory::PlayerInventory>),
    >,
) {
    for (inventory, mut audio) in &mut players {
        let equipped = core::array::from_fn(|index| {
            inventory
                .slot(crate::inventory::ARMOR_START + index)
                .map(|stack| stack.item())
        });
        if let Some(previous) = audio.0 {
            for (before, after) in previous.into_iter().zip(equipped) {
                if before == after {
                    continue;
                }
                if let Some(sound) = after.and_then(crate::inventory::catalog::equip_sound) {
                    commands.trigger(Sound::event(sound));
                }
            }
        }
        audio.0 = Some(equipped);
    }
}
