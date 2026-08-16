use bevy::prelude::*;

#[derive(Default)]
pub struct SpriteAnimationPlugin;

#[derive(Component, Default, Clone, Deref, DerefMut)]
pub struct SpriteAnimationTimer(pub Timer);

#[derive(Component, Default, Clone)]
pub struct SpriteAnimationIndices {
    pub start: u16,
    pub end: u16,
}

impl SpriteAnimationIndices {
    pub fn new(start: u16, end: u16) -> Self {
        SpriteAnimationIndices { start, end }
    }
}

impl Plugin for SpriteAnimationPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, sprite_animation_system);
    }
}

fn sprite_animation_system(
    mut query: Query<(
        &mut SpriteAnimationTimer,
        &SpriteAnimationIndices,
        &mut Sprite,
    )>,
    time: Res<Time>,
) {
    for (mut timer, indices, mut sprite) in &mut query {
        timer.tick(time.delta());
        let Some(atlas) = &mut sprite.texture_atlas else {
            continue;
        };
        if atlas.index < indices.start as usize {
            atlas.index = indices.start as usize;
        }
        if !timer.just_finished() {
            continue;
        }

        atlas.index += 1;
        if atlas.index > indices.end as usize {
            atlas.index = indices.start as usize;
        }
    }
}
