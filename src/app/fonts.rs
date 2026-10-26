//! Bundle complete text fonts independently of the platform's installed fonts.
use bevy::{asset::AssetId, prelude::*};

pub(super) fn install(mut fonts: ResMut<Assets<Font>>) {
    fonts
        .insert(
            AssetId::default(),
            Font::from_bytes(include_bytes!("../../assets/fonts/NotoSans.ttf").to_vec()),
        )
        .expect("default font asset");
}

/// Feathers supplies its own font handles. Replace them once when text appears,
/// while preserving each widget's size, weight, and inherited colors.
pub(super) fn style_new_text(
    mut text: Query<&mut TextFont>,
    mut symbol_font: Local<Option<Handle<Font>>>,
    mut fonts: ResMut<Assets<Font>>,
) {
    let symbols = symbol_font.get_or_insert_with(|| {
        fonts.add(Font::from_bytes(
            include_bytes!("../../assets/fonts/NotoSansSymbols2.ttf").to_vec(),
        ))
    });
    for mut font in &mut text {
        if font.is_changed() && !matches!(font.font, FontSource::List(_)) {
            font.font = FontSource::List(vec![
                Handle::<Font>::default().into(),
                symbols.clone().into(),
            ]);
        }
    }
}
