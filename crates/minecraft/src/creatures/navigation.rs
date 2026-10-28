use super::*;
use crate::{
    character::{Aabb, CharacterBody, CharacterMovement, CharacterState, Terrain},
    simulation::{GameplayClock, Simulating},
};
use bevy::{math::DVec3, prelude::*};
use hollow_features::{
    actors::{Behavior, Decision, Navigation, NavigationStatus, PathHandle},
    motion::FixedPosition,
};
use std::collections::VecDeque;

/// Opt into the voxel provider independently of species and decision policy.
#[derive(Component, Default)]
#[require(
    crate::character::GameCharacter,
    Decision,
    Navigation,
    SegmentValidation
)]
pub struct VoxelNavigator;

const WIDTH: usize = 33;
const HEIGHT: usize = 17;
const CELLS: usize = WIDTH * HEIGHT * WIDTH;
const NONE: u16 = u16::MAX;
const MAX_NODES: usize = 4096;
const MAX_PATHS: usize = 16_384;
const SEARCH_SLOTS: usize = 8;
const FOLLOW_PROBES: usize = 16_384;
const SEARCH_PROBES: usize = 16_384;
const EXPANSIONS: usize = 4096;
const DIRECTIONS: [IVec3; 4] = [IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z];

#[derive(Clone, Copy)]
struct Node {
    generation: u32,
    cost: u16,
    parent: u16,
    feet: f32,
    heap: u16,
    closed: bool,
}
impl Default for Node {
    fn default() -> Self {
        Self {
            generation: 0,
            cost: NONE,
            parent: NONE,
            feet: 0.0,
            heap: NONE,
            closed: false,
        }
    }
}

// An indexed heap holds one frontier entry per node: decreasing a cost never
// creates stale entries, grows the frontier beyond its bound, or allocates.
struct Search {
    entity: Option<Entity>,
    destination: Vec3,
    origin: IVec3,
    goal: IVec3,
    escape: bool,
    nodes: Box<[Node]>,
    open: Vec<u16>,
    visited: usize,
    generation: u32,
    terrain: Terrain,
    completed: Option<u16>,
}
impl Default for Search {
    fn default() -> Self {
        Self {
            entity: None,
            destination: Vec3::ZERO,
            origin: IVec3::ZERO,
            goal: IVec3::ZERO,
            escape: false,
            nodes: vec![Node::default(); CELLS].into_boxed_slice(),
            open: Vec::with_capacity(MAX_NODES),
            visited: 0,
            generation: 0,
            terrain: Terrain::default(),
            completed: None,
        }
    }
}
impl Search {
    fn touch(&mut self, index: u16) {
        let node = &mut self.nodes[index as usize];
        if node.generation != self.generation {
            *node = Node {
                generation: self.generation,
                ..default()
            };
        }
    }
    fn index(&self, p: IVec3) -> Option<u16> {
        let p = p - self.origin;
        (p.x >= 0
            && p.x < WIDTH as i32
            && p.z >= 0
            && p.z < WIDTH as i32
            && p.y >= 0
            && p.y < HEIGHT as i32)
            .then(|| (p.x as usize + WIDTH * (p.z as usize + WIDTH * p.y as usize)) as u16)
    }
    fn position(&self, index: u16) -> IVec3 {
        let i = index as usize;
        self.origin
            + IVec3::new(
                (i % WIDTH) as i32,
                (i / (WIDTH * WIDTH)) as i32,
                (i / WIDTH % WIDTH) as i32,
            )
    }
    fn point(&self, index: u16) -> DVec3 {
        let p = self.position(index);
        DVec3::new(
            p.x as f64 + 0.5,
            self.nodes[index as usize].feet as f64,
            p.z as f64 + 0.5,
        )
    }
    fn priority(&self, index: u16) -> u32 {
        let p = self.position(index);
        let h = if self.escape {
            0
        } else {
            (p.x - self.goal.x).unsigned_abs() + (p.z - self.goal.z).unsigned_abs()
        };
        self.nodes[index as usize].cost as u32 + h * 10
    }
    fn swap(&mut self, a: usize, b: usize) {
        self.open.swap(a, b);
        self.nodes[self.open[a] as usize].heap = a as u16;
        self.nodes[self.open[b] as usize].heap = b as u16;
    }
    fn push_or_decrease(&mut self, index: u16) {
        let mut i = self.nodes[index as usize].heap as usize;
        if i == NONE as usize {
            i = self.open.len();
            self.open.push(index);
            self.nodes[index as usize].heap = i as u16;
        }
        while i > 0 {
            let parent = (i - 1) / 2;
            if self.priority(self.open[parent]) <= self.priority(self.open[i]) {
                break;
            }
            self.swap(i, parent);
            i = parent;
        }
    }
    fn pop(&mut self) -> Option<u16> {
        let result = *self.open.first()?;
        let last = self.open.pop().unwrap();
        self.nodes[result as usize].heap = NONE;
        if !self.open.is_empty() {
            self.open[0] = last;
            self.nodes[last as usize].heap = 0;
            let mut i = 0;
            loop {
                let left = i * 2 + 1;
                if left >= self.open.len() {
                    break;
                }
                let right = left + 1;
                let child = if right < self.open.len()
                    && self.priority(self.open[right]) < self.priority(self.open[left])
                {
                    right
                } else {
                    left
                };
                if self.priority(self.open[i]) <= self.priority(self.open[child]) {
                    break;
                }
                self.swap(i, child);
                i = child;
            }
        }
        Some(result)
    }
    fn begin(&mut self, entity: Entity, feet: Vec3, destination: Vec3, escape: bool) {
        self.entity = Some(entity);
        self.completed = None;
        self.destination = destination;
        let start = feet.floor().as_ivec3();
        self.origin = start - IVec3::new(16, 8, 16);
        self.goal = destination
            .floor()
            .as_ivec3()
            .clamp(self.origin, self.origin + IVec3::new(32, 16, 32));
        self.escape = escape;
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.nodes.fill(Node::default());
            self.generation = 1;
        }
        self.open.clear();
        self.visited = 1;
        let index = self.index(start).unwrap();
        self.touch(index);
        self.nodes[index as usize].cost = 0;
        self.nodes[index as usize].feet = feet.y;
        self.push_or_decrease(index);
    }
    /// Returns false while the shared path pool is full.
    fn finish(
        &mut self,
        end: u16,
        body: &CharacterBody,
        navigation: &mut Navigation,
        pool: &mut PathPool,
    ) -> bool {
        let Some(handle) = pool.allocate() else {
            return false;
        };
        // Any path the bounded search box admits is valid; slots retain their buffers.
        let points = &mut pool.slots[handle.0 as usize].points;
        let mut index = end;
        while self.nodes[index as usize].parent != NONE {
            points.push(self.point(index).as_vec3() + Vec3::Y * body.origin_height);
            index = self.nodes[index as usize].parent;
        }
        points.reverse();
        navigation.path = Some(handle);
        navigation.cursor = 0;
        navigation.status = NavigationStatus::Following;
        true
    }
}

#[derive(Default)]
struct PathSlot {
    points: Vec<Vec3>,
    in_use: bool,
}

/// Slots retain bounded buffers across routes and use a dense numeric handle.
#[derive(Resource, Default)]
pub(crate) struct PathPool {
    slots: Vec<PathSlot>,
    free: Vec<u32>,
}
impl PathPool {
    fn allocate(&mut self) -> Option<PathHandle> {
        let index = if let Some(index) = self.free.pop() {
            index
        } else {
            if self.slots.len() == MAX_PATHS {
                return None;
            }
            self.slots.push(PathSlot::default());
            (self.slots.len() - 1) as u32
        };
        self.slots[index as usize].in_use = true;
        Some(PathHandle(index))
    }
    fn release(&mut self, handle: PathHandle) {
        if let Some(slot) = self.slots.get_mut(handle.0 as usize)
            && slot.in_use
        {
            slot.in_use = false;
            slot.points.clear();
            self.free.push(handle.0);
        }
    }
    fn get(&self, handle: PathHandle) -> Option<&[Vec3]> {
        self.slots
            .get(handle.0 as usize)
            .filter(|slot| slot.in_use)
            .map(|slot| slot.points.as_slice())
    }
}

fn release_removed_navigation(
    event: On<Remove<Navigation>>,
    actors: Query<&Navigation>,
    mut pool: ResMut<PathPool>,
) {
    if let Ok(navigation) = actors.get(event.entity)
        && let Some(handle) = navigation.path
    {
        pool.release(handle);
    }
}

/// One validated segment avoids collecting unchanged voxel shapes every tick.
/// Four column revision keys and two endpoints cost about 128 bytes per navigator.
#[derive(Component, Default)]
pub(crate) struct SegmentValidation {
    from: Vec3,
    to: Vec3,
    revisions: [Option<(Entity, u64)>; 4],
    valid: bool,
    escape: bool,
    blocked_ticks: u8,
    jump_cooldown: u8,
    queued: bool,
}
fn revisions(
    world: &crate::voxel::VoxelWorld,
    from: Vec3,
    to: Vec3,
    body: &CharacterBody,
) -> [Option<(Entity, u64)>; 4] {
    let min = crate::voxel::chunk_at(from.min(to) - Vec3::splat(body.half_width));
    let max = crate::voxel::chunk_at(from.max(to) + Vec3::splat(body.half_width));
    let a = world.column_revision(min);
    let x = if min.x == max.x {
        a
    } else {
        world.column_revision(IVec2::new(max.x, min.y))
    };
    let z = if min.y == max.y {
        a
    } else {
        world.column_revision(IVec2::new(min.x, max.y))
    };
    let last = if min.x == max.x {
        z
    } else if min.y == max.y {
        x
    } else {
        world.column_revision(max)
    };
    [a, x, z, last]
}
const _: () =
    assert!(SEARCH_SLOTS * (CELLS * std::mem::size_of::<Node>() + MAX_NODES * 2) < 4 * 1024 * 1024);

#[derive(Resource)]
struct Searches([Search; SEARCH_SLOTS]);
#[derive(Resource, Default)]
struct SearchQueue(VecDeque<(Entity, u64)>);

fn clear_searches(mut queue: ResMut<SearchQueue>, mut searches: ResMut<Searches>) {
    queue.0.clear();
    for search in &mut searches.0 {
        search.entity = None;
    }
}
impl Default for Searches {
    fn default() -> Self {
        // Direct node tables and indexed heaps stay below the 4 MiB ceiling.
        Self(std::array::from_fn(|_| Search::default()))
    }
}
#[derive(Resource, Default)]
pub(crate) struct NavigationBudget {
    expansions: usize,
    follow_probes: usize,
    search_probes: usize,
}
fn reset(
    mut budget: ResMut<NavigationBudget>,
    mut diagnostics: ResMut<crate::simulation::SimulationDiagnostics>,
) {
    diagnostics.navigation_expansions = 0;
    diagnostics.navigation_probes = 0;
    budget.expansions = EXPANSIONS;
    budget.follow_probes = FOLLOW_PROBES;
    budget.search_probes = SEARCH_PROBES;
}

/// Charge the exact cells visited by Terrain (including its fence support row).
fn gather(
    world: &crate::voxel::VoxelWorld,
    terrain: &mut Terrain,
    feet: DVec3,
    body: &CharacterBody,
    probes: &mut usize,
) -> bool {
    let reach = body.half_width as f64 + 1.01;
    let region = Aabb {
        min: feet - DVec3::new(reach, 3.01, reach),
        max: feet
            + DVec3::new(
                reach,
                body.height(crate::character::Pose::Standing) as f64 + 1.26,
                reach,
            ),
    };
    let size = region.max.floor().as_ivec3() - region.min.floor().as_ivec3() + IVec3::new(1, 2, 1);
    let cost = (size.x * size.y * size.z) as usize;
    if cost > *probes {
        return false;
    }
    *probes -= cost;
    terrain.gather(world, region);
    true
}

enum SearchOutcome {
    Pending,
    Failed,
    Found(u16),
}

struct SearchProgress {
    entity: Entity,
    outcome: SearchOutcome,
    expansions: usize,
    probes: usize,
}

/// Each search owns its scratch space; workers only read the shared voxel world.
fn advance_search(
    search: &mut Search,
    world: &crate::voxel::VoxelWorld,
    body: CharacterBody,
    expansion_limit: usize,
    probe_limit: usize,
) -> SearchProgress {
    let entity = search.entity.expect("Only active searches are advanced");
    if let Some(end) = search.completed {
        return SearchProgress {
            entity,
            outcome: SearchOutcome::Found(end),
            expansions: 0,
            probes: 0,
        };
    }
    let mut terrain = std::mem::take(&mut search.terrain);
    let mut probes = probe_limit;
    let mut expansions = 0;
    let mut outcome = SearchOutcome::Pending;
    while expansions < expansion_limit {
        let Some(&head) = search.open.first() else {
            outcome = SearchOutcome::Failed;
            break;
        };
        let from = search.point(head);
        if !gather(world, &mut terrain, from, &body, &mut probes) {
            break;
        }
        expansions += 1;
        search.pop();
        search.nodes[head as usize].closed = true;
        let p = search.position(head);
        let dry = !terrain.has_fluid(&body.aabb(
            from + DVec3::Y * body.origin_height as f64,
            crate::character::Pose::Standing,
        ));
        if (search.escape && dry)
            || (!search.escape
                && (p.x - search.goal.x).abs() + (p.z - search.goal.z).abs() <= 1
                && (p.y - search.goal.y).abs() <= 3)
        {
            search.completed = Some(head);
            outcome = SearchOutcome::Found(head);
            break;
        }
        for direction in DIRECTIONS {
            let target = from + direction.as_dvec3();
            let Some((y, wet)) = terrain.navigation_step(from, target, &body, search.escape) else {
                continue;
            };
            let Some(index) = search.index(target.with_y(y).floor().as_ivec3()) else {
                continue;
            };
            search.touch(index);
            let old = search.nodes[index as usize];
            if old.closed {
                continue;
            }
            let cost = search.nodes[head as usize]
                .cost
                .saturating_add(10 + if wet { 30 } else { 0 } + ((y - from.y).abs() * 4.0) as u16);
            if cost >= old.cost {
                continue;
            }
            if old.cost == NONE {
                if search.visited >= MAX_NODES {
                    continue;
                }
                search.visited += 1;
            }
            let node = &mut search.nodes[index as usize];
            node.cost = cost;
            node.parent = head;
            node.feet = y as f32;
            search.push_or_decrease(index);
        }
    }
    search.terrain = terrain;
    SearchProgress {
        entity,
        outcome,
        expansions,
        probes: probe_limit - probes,
    }
}

impl Plugin for CreatureNavigationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Searches>()
            .init_resource::<SearchQueue>()
            .init_resource::<PathPool>()
            .init_resource::<NavigationBudget>()
            .add_observer(release_removed_navigation)
            .add_systems(OnExit(crate::game::GameState::Game), clear_searches)
            .add_systems(
                RunFixedMainLoop,
                reset.in_set(bevy::app::RunFixedMainLoopSystems::BeforeFixedMainLoop),
            )
            .add_systems(
                FixedUpdate,
                (follow, search).chain().in_set(CreatureSystems::Navigate),
            );
    }
}

fn fail(navigation: &mut Navigation, tick: u64, pool: &mut PathPool) {
    if let Some(handle) = navigation.path.take() {
        pool.release(handle);
    }
    navigation.status = NavigationStatus::Failed;
    navigation.retry_tick = tick + 20;
}

fn follow(
    world: crate::voxel::VoxelWorld,
    clock: Res<GameplayClock>,
    mut budget: ResMut<NavigationBudget>,
    mut pool: ResMut<PathPool>,
    mut terrain: Local<Terrain>,
    mut actors: Query<
        (
            &FixedPosition,
            &CharacterBody,
            &CharacterState,
            &Decision,
            &mut Navigation,
            &mut CharacterMovement,
            &mut SegmentValidation,
        ),
        (
            With<VoxelNavigator>,
            With<Simulating>,
            Without<super::combat::Dying>,
        ),
    >,
) {
    for (position, body, state, decision, mut navigation, mut movement, mut validation) in
        &mut actors
    {
        if navigation.status != NavigationStatus::Following
            && let Some(handle) = navigation.path.take()
        {
            pool.release(handle);
        }
        let mut next = CharacterMovement {
            jump: state.in_swimmable_fluid,
            ..default()
        };
        validation.jump_cooldown = validation.jump_cooldown.saturating_sub(1);
        if state.horizontal_collision
            && (position.current - position.previous).xz().length_squared() < 0.000025
        {
            validation.blocked_ticks = validation.blocked_ticks.saturating_add(1);
        } else {
            validation.blocked_ticks = 0;
        }
        if validation.blocked_ticks >= 10 && navigation.status == NavigationStatus::Following {
            validation.valid = false;
            fail(&mut navigation, clock.0, &mut pool);
        }
        if navigation.status == NavigationStatus::Failed && clock.0 >= navigation.retry_tick {
            navigation.status = NavigationStatus::Pending;
        }
        if navigation.status == NavigationStatus::Following {
            let Some(path) = navigation.path.and_then(|handle| pool.get(handle)) else {
                fail(&mut navigation, clock.0, &mut pool);
                movement.set_if_neq(next);
                continue;
            };
            let current = position.current.as_vec3();
            while navigation.cursor < path.len() {
                let target = path[navigation.cursor];
                if (target - current).with_y(0.0).length_squared() > 0.09
                    || (target.y - current.y).abs() > 0.6
                {
                    break;
                }
                navigation.cursor += 1;
            }
            if navigation.cursor == path.len() {
                navigation.set_destination(None);
                if let Some(handle) = navigation.path.take() {
                    pool.release(handle);
                }
            } else {
                let target = path[navigation.cursor];
                let feet = position.current - DVec3::Y * body.origin_height as f64;
                let delta = (target - validation.from).with_y(0.0);
                let t = if delta.length_squared() > 0.0001 {
                    (current - validation.from).with_y(0.0).dot(delta) / delta.length_squared()
                } else {
                    0.0
                };
                let corridor = validation.from + delta * t.clamp(0.0, 1.0);
                let unchanged = validation.valid
                    && validation.to == target
                    && validation.escape == (decision.behavior == Behavior::EscapeWater)
                    && (current - corridor).with_y(0.0).length_squared() < 0.09
                    && (current.y - validation.from.y).abs() < 3.1
                    && validation.revisions == revisions(&world, validation.from, target, body);
                let close = (target - current).with_y(0.0).length_squared() <= 4.0;
                let can_probe = !unchanged
                    && close
                    && gather(&world, &mut terrain, feet, body, &mut budget.follow_probes);
                let valid = unchanged
                    || (can_probe
                        && terrain
                            .navigation_step(
                                feet,
                                target.as_dvec3(),
                                body,
                                decision.behavior == Behavior::EscapeWater,
                            )
                            .is_some_and(|(y, _)| {
                                (y + body.origin_height as f64 - target.y as f64).abs() < 0.1
                            }));
                if valid && !unchanged {
                    validation.from = current;
                    validation.to = target;
                    validation.revisions = revisions(&world, current, target, body);
                    validation.valid = true;
                    validation.escape = decision.behavior == Behavior::EscapeWater;
                }
                if valid || (close && !state.on_ground && !state.in_fluid) {
                    next.direction = (target - current).with_y(0.0).normalize_or_zero();
                    next.look_direction = next.direction;
                    // Animals flee faster, but do not use the player's sprint-jump boost.
                    if target.y > current.y + body.step_height as f32
                        && state.on_ground
                        && validation.jump_cooldown == 0
                    {
                        next.jump = true;
                        validation.jump_cooldown = 10;
                    }
                } else if can_probe || !close {
                    validation.valid = false;
                    fail(&mut navigation, clock.0, &mut pool);
                }
            }
        }
        movement.set_if_neq(next);
    }
}

fn search(
    world: crate::voxel::VoxelWorld,
    clock: Res<GameplayClock>,
    mut budget: ResMut<NavigationBudget>,
    mut searches: ResMut<Searches>,
    mut queue: ResMut<SearchQueue>,
    mut pool: ResMut<PathPool>,
    mut diagnostics: ResMut<crate::simulation::SimulationDiagnostics>,
    mut actors: Query<
        (
            Entity,
            &FixedPosition,
            &CharacterBody,
            &Decision,
            &mut Navigation,
            &mut SegmentValidation,
            Has<Simulating>,
            Has<super::combat::Dying>,
        ),
        With<VoxelNavigator>,
    >,
) {
    for search in &mut searches.0 {
        if let Some(entity) = search.entity {
            let valid =
                actors
                    .get(entity)
                    .is_ok_and(|(_, _, _, decision, nav, _, active, dying)| {
                        active
                            && !dying
                            && nav.status == NavigationStatus::Pending
                            && nav.destination == Some(search.destination)
                            && search.escape == (decision.behavior == Behavior::EscapeWater)
                    });
            if !valid {
                search.entity = None;
            }
        }
    }
    // The component flag deduplicates requests without a second spatial index.
    // Stale entries are discarded when dequeued; a bound prevents unbounded churn.
    for (entity, _, _, _, navigation, mut validation, active, dying) in &mut actors {
        if !active
            || dying
            || navigation.status != NavigationStatus::Pending
            || validation.queued
            || searches.0.iter().any(|s| s.entity == Some(entity))
        {
            continue;
        }
        if queue.0.len() >= 32_768 {
            break;
        }
        validation.queued = true;
        queue.0.push_back((entity, clock.0));
    }
    while let Some(slot) = searches.0.iter_mut().find(|s| s.entity.is_none()) {
        let Some((entity, _)) = queue.0.pop_front() else {
            break;
        };
        let Ok((_, position, body, decision, navigation, mut validation, active, dying)) =
            actors.get_mut(entity)
        else {
            continue;
        };
        validation.queued = false;
        if active
            && !dying
            && navigation.status == NavigationStatus::Pending
            && let Some(destination) = navigation.destination
        {
            slot.begin(
                entity,
                position.current.as_vec3() - Vec3::Y * body.origin_height,
                destination - Vec3::Y * body.origin_height,
                decision.behavior == Behavior::EscapeWater,
            );
            // Keep controller-space destination for cancellation comparison.
            slot.destination = destination;
        }
    }
    // Divide the frame budget before dispatch, so workers never contend on it.
    let bodies: [Option<CharacterBody>; SEARCH_SLOTS] = std::array::from_fn(|index| {
        searches.0[index].entity.and_then(|entity| {
            actors
                .get(entity)
                .ok()
                .map(|(_, _, body, _, _, _, _, _)| *body)
        })
    });
    let active = bodies.iter().filter(|body| body.is_some()).count();
    let mut results: [Option<SearchProgress>; SEARCH_SLOTS] = std::array::from_fn(|_| None);
    if active > 0 && budget.expansions > 0 && budget.search_probes > 0 {
        let expansions = budget.expansions / active;
        let probes = budget.search_probes / active;
        if active == 1 {
            let index = bodies.iter().position(Option::is_some).unwrap();
            results[index] = Some(advance_search(
                &mut searches.0[index],
                &world,
                bodies[index].unwrap(),
                expansions,
                probes,
            ));
        } else {
            let world = &world;
            let completed = bevy::tasks::ComputeTaskPool::get().scope(|scope| {
                for (index, (search, body)) in searches.0.iter_mut().zip(bodies).enumerate() {
                    if let Some(body) = body {
                        scope.spawn(async move {
                            (
                                index,
                                advance_search(search, world, body, expansions, probes),
                            )
                        });
                    }
                }
            });
            for (index, progress) in completed {
                results[index] = Some(progress);
            }
        }
    }
    for (search, progress) in searches
        .0
        .iter_mut()
        .zip(results)
        .filter_map(|(search, result)| result.map(|progress| (search, progress)))
    {
        budget.expansions -= progress.expansions;
        budget.search_probes -= progress.probes;
        diagnostics.navigation_expansions += progress.expansions;
        let Ok((_, _, body, _, mut navigation, _, _, _)) = actors.get_mut(progress.entity) else {
            search.entity = None;
            continue;
        };
        match progress.outcome {
            SearchOutcome::Pending => {}
            SearchOutcome::Failed => {
                fail(&mut navigation, clock.0, &mut pool);
                search.entity = None;
            }
            SearchOutcome::Found(end) => {
                if search.finish(end, body, &mut navigation, &mut pool) {
                    search.entity = None;
                }
            }
        }
    }
    diagnostics.navigation_pending = actors
        .iter()
        .filter(|(_, _, _, _, n, _, active, dying)| {
            *active && !*dying && n.status == NavigationStatus::Pending
        })
        .count();
    diagnostics.navigation_oldest_wait = queue
        .0
        .front()
        .map_or(0, |(_, queued)| clock.0.saturating_sub(*queued));
    diagnostics.navigation_queue_bytes = queue.0.capacity() * std::mem::size_of::<(Entity, u64)>();
    diagnostics.navigation_search_bytes = searches
        .0
        .iter()
        .map(|search| {
            search.nodes.len() * std::mem::size_of::<Node>()
                + search.open.capacity() * std::mem::size_of::<u16>()
        })
        .sum();
    diagnostics.navigation_probes =
        FOLLOW_PROBES + SEARCH_PROBES - budget.follow_probes - budget.search_probes;
}
