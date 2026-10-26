use super::{InventorySlotView, SlotCount, SlotIcon, refresh_slots, return_cursor_stack};
use crate::{
    game::GameState,
    inventory::{INVENTORY_SIZE, InventoryCursor, ItemKind, PlayerInventory, icons::BlockIcons},
    player::Player,
};
use bevy::prelude::*;

#[derive(Resource, Default)]
struct RecordedDrops(Vec<crate::inventory::ItemStack>);

#[test]
fn ending_session_clears_cursor_without_spawning_orphaned_drops() {
    use crate::{dropped_item::SpawnDroppedItem, inventory::ItemStack, voxel::VoxelKind};
    use bevy::ecs::system::RunSystemOnce;
    let mut world = World::new();
    world.insert_resource(State::new(GameState::Menu));
    world.init_resource::<InventoryCursor>();
    world.init_resource::<crate::inventory::access::InventoryActions>();
    world.init_resource::<RecordedDrops>();
    world.add_observer(
        |event: On<SpawnDroppedItem>, mut drops: ResMut<RecordedDrops>| drops.0.push(event.stack),
    );
    world.spawn(Player);
    world.resource_mut::<InventoryCursor>().stack =
        Some(ItemStack::new(ItemKind::Block(VoxelKind::Dirt), 1));
    world.run_system_once(return_cursor_stack).unwrap();
    world
        .run_system_once(crate::inventory::reset_inventory_cursor)
        .unwrap();
    assert!(world.resource::<InventoryCursor>().stack.is_none());
    assert!(world.resource::<RecordedDrops>().0.is_empty());
}

#[test]
fn closing_full_inventory_drops_remainder_without_losing_items() {
    use crate::{dropped_item::SpawnDroppedItem, inventory::ItemStack, voxel::VoxelKind};
    use bevy::ecs::system::RunSystemOnce;
    let mut world = World::new();
    world.insert_resource(State::new(GameState::Game));
    world.init_resource::<InventoryCursor>();
    world.init_resource::<crate::inventory::access::InventoryActions>();
    world.init_resource::<RecordedDrops>();
    world.add_observer(
        |event: On<SpawnDroppedItem>, mut drops: ResMut<RecordedDrops>| drops.0.push(event.stack),
    );
    let mut inventory = PlayerInventory::default();
    inventory.insert(ItemStack::new(ItemKind::Block(VoxelKind::Dirt), 62));
    for _ in 1..INVENTORY_SIZE {
        assert!(
            inventory
                .insert(ItemStack::new(ItemKind::Block(VoxelKind::Stone), 64))
                .is_none()
        );
    }
    let mut held = None;
    crate::inventory::slots::click(&mut inventory, 0, &mut held, false);
    inventory.insert(ItemStack::new(ItemKind::Block(VoxelKind::Wood), 1));
    let player = world
        .spawn((
            Player,
            inventory,
            GlobalTransform::from_translation(Vec3::Y),
        ))
        .id();
    world.resource_mut::<InventoryCursor>().stack = held;
    let before: usize = (0..INVENTORY_SIZE)
        .filter_map(|i| world.get::<PlayerInventory>(player).unwrap().slot(i))
        .map(|s| s.count() as usize)
        .sum::<usize>()
        + held.unwrap().count() as usize;
    world.run_system_once(return_cursor_stack).unwrap();
    assert!(world.resource::<InventoryCursor>().stack.is_none());
    assert_eq!(
        world.resource::<RecordedDrops>().0.as_slice(),
        &[held.unwrap()]
    );
    let after: usize = (0..INVENTORY_SIZE)
        .filter_map(|i| world.get::<PlayerInventory>(player).unwrap().slot(i))
        .map(|s| s.count() as usize)
        .sum::<usize>()
        + world
            .resource::<RecordedDrops>()
            .0
            .iter()
            .map(|s| s.count() as usize)
            .sum::<usize>();
    assert_eq!(before, after);
}

#[test]
fn slots_initialize_when_opened_and_remain_unchanged_at_rest() {
    let mut app = App::new();
    app.init_resource::<BlockIcons>()
        .add_systems(Update, refresh_slots);
    let player = app.world_mut().spawn(Player).id();
    let view = app
        .world_mut()
        .spawn((
            InventorySlotView { index: 0 },
            super::SlotOwner {
                entity: player,
                index: 0,
            },
            BorderColor::all(Color::NONE),
        ))
        .id();
    let icon = app
        .world_mut()
        .spawn((
            SlotIcon,
            ChildOf(view),
            ImageNode::default(),
            Visibility::Hidden,
        ))
        .id();
    let count = app
        .world_mut()
        .spawn((SlotCount, Text::new(""), ChildOf(view)))
        .id();
    app.update();
    assert_eq!(app.world().get::<Text>(count).unwrap().0, "2");
    let text_tick = app
        .world()
        .entity(count)
        .get_change_ticks::<Text>()
        .unwrap()
        .changed;
    let icon_tick = app
        .world()
        .entity(icon)
        .get_change_ticks::<ImageNode>()
        .unwrap()
        .changed;
    let border_tick = app
        .world()
        .entity(view)
        .get_change_ticks::<BorderColor>()
        .unwrap()
        .changed;
    app.update();
    assert_eq!(
        app.world()
            .entity(count)
            .get_change_ticks::<Text>()
            .unwrap()
            .changed,
        text_tick
    );
    assert_eq!(
        app.world()
            .entity(icon)
            .get_change_ticks::<ImageNode>()
            .unwrap()
            .changed,
        icon_tick
    );
    assert_eq!(
        app.world()
            .entity(view)
            .get_change_ticks::<BorderColor>()
            .unwrap()
            .changed,
        border_tick
    );
    let new_count = app
        .world_mut()
        .spawn((SlotCount, Text::new(""), ChildOf(view)))
        .id();
    app.world_mut().spawn((
        InventorySlotView { index: 0 },
        BorderColor::all(Color::NONE),
    ));
    app.update();
    assert_eq!(app.world().get::<Text>(new_count).unwrap().0, "2");
    assert_eq!(
        app.world()
            .entity(count)
            .get_change_ticks::<Text>()
            .unwrap()
            .changed,
        text_tick
    );
}
