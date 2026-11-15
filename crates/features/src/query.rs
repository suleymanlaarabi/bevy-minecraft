//! Selective writes to dense query columns without dirtying the entire table.
use bevy_ecs::change_detection::ContiguousMut;

/// Preserve per-component change detection when updating a contiguous column.
#[inline]
pub fn set_if_neq<T: PartialEq>(values: &mut ContiguousMut<'_, T>, index: usize, value: T) {
    set_field_if_neq(values, index, |value| value, value);
}

/// Compare only the field being updated, preserving the rest of the component.
#[inline]
pub fn set_field_if_neq<T, U: PartialEq>(
    values: &mut ContiguousMut<'_, T>,
    index: usize,
    field: impl FnOnce(&mut T) -> &mut U,
    value: U,
) {
    let current = field(&mut values.bypass_change_detection()[index]);
    if *current != value {
        *current = value;
        // Bevy table lengths fit in u32; an in-bounds index leaves room for the end.
        let row = u32::try_from(index).expect("Component index must fit in a table row");
        values.reborrow().slice(row..row + 1).mark_all_as_changed();
    }
}
