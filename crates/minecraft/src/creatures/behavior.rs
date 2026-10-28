use super::*;
use crate::{
    character::CharacterState,
    simulation::{GameplayClock, Simulating},
    survival::DamageApplied,
};
use bevy::prelude::*;
use hollow_features::{
    actors::{Behavior, Decision, Navigation, NavigationStatus, ThreatMemory},
    motion::FixedPosition,
};

impl Plugin for CreatureBehaviorPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            (update_threats, decide)
                .chain()
                .in_set(CreatureSystems::Decide),
        )
        .add_observer(remember_threat);
    }
}

fn update_threats(
    clock: Res<GameplayClock>,
    targets: Query<&FixedPosition>,
    mut actors: Query<
        (
            &Decision,
            &CharacterState,
            &mut ThreatMemory,
            Has<EscapeWater>,
        ),
        (With<Flee>, With<Simulating>, Without<super::combat::Dying>),
    >,
) {
    actors
        .par_iter_mut()
        .for_each(|(decision, state, mut threat, escape)| {
            let water = escape
                && state.in_swimmable_fluid
                && (!state.on_ground || state.eyes_in_swimmable_fluid);
            if !water && decision.next_tick > clock.0 {
                return;
            }
            if threat.expires <= clock.0 {
                if threat.target.is_some() {
                    threat.target = None;
                }
            } else if let Some(target) = threat.target {
                if let Ok(target) = targets.get(target) {
                    let position = target.current.as_vec3();
                    if threat.position != position {
                        threat.position = position;
                    }
                } else {
                    threat.target = None;
                }
            }
        });
}

fn decide(
    clock: Res<GameplayClock>,
    session: Res<hollow_worldgen::GenerationSession>,
    mut actors: Query<
        (
            Entity,
            &FixedPosition,
            &CharacterState,
            &mut Decision,
            Option<&ThreatMemory>,
            &mut Navigation,
            Has<Wander>,
            Has<Flee>,
            Has<EscapeWater>,
        ),
        (
            Or<(With<Wander>, With<Flee>, With<EscapeWater>)>,
            With<Simulating>,
            Without<super::combat::Dying>,
        ),
    >,
) {
    let seed = session.settings().seed as u64;
    actors.par_iter_mut().for_each(
        |(entity, position, state, mut decision, threat, mut navigation, wander, flee, escape)| {
            let water = escape
                && state.in_swimmable_fluid
                && (!state.on_ground || state.eyes_in_swimmable_fluid);
            let threatened = flee
                && threat
                    .as_ref()
                    .is_some_and(|threat| threat.expires > clock.0);
            if !water && decision.next_tick > clock.0 {
                return;
            }
            if decision.next_tick == 0 && !water && !threatened {
                decision.next_tick = clock.0 + entity.index().index() as u64 % 10 + 1;
                return;
            }
            decision.next_tick = clock.0 + 10;
            let feet = position.current.as_vec3();
            // Decision randomness is local to this actor and tick, so workers share no mutable RNG.
            let mut random = CreatureRandom(
                (seed ^ entity.to_bits() ^ clock.0.wrapping_mul(0x9e3779b97f4a7c15)).max(1),
            );
            let desired = if water {
                Behavior::EscapeWater
            } else if threatened {
                Behavior::Flee
            } else if wander && (navigation.destination.is_some() || random.range(5) == 0) {
                Behavior::Wander
            } else {
                Behavior::Idle
            };
            let changed = decision.behavior != desired;
            decision.behavior = desired;
            if desired == Behavior::Idle {
                navigation.set_destination(None);
            } else if desired == Behavior::Flee {
                let threat = threat.expect("Flee requires an active threat");
                let away = (feet - threat.position).with_y(0.0).normalize_or(Vec3::X);
                let destination = feet + away * 10.0;
                // Reuse the escape route until it ends or the threat cuts it off.
                let route_toward_threat = navigation.destination.is_some_and(|goal| {
                    (threat.position - feet).with_y(0.0).length_squared() < 9.0
                        && (goal - feet).with_y(0.0).dot(away) < 0.0
                });
                if changed || navigation.destination.is_none() || route_toward_threat {
                    navigation.set_destination(Some(destination));
                }
            } else if changed || navigation.destination.is_none() {
                let offset = Vec3::new(
                    random.range(21) as f32 - 10.0,
                    0.0,
                    random.range(21) as f32 - 10.0,
                );
                navigation.set_destination(Some(feet + offset));
            } else if navigation.status == NavigationStatus::Failed && desired == Behavior::Wander {
                navigation.set_destination(None);
            }
        },
    );
}

fn remember_threat(
    impact: On<DamageApplied>,
    clock: Res<GameplayClock>,
    positions: Query<&FixedPosition>,
    mut actors: Query<(&mut ThreatMemory, &mut Decision), With<Flee>>,
) {
    if impact.fatal {
        return;
    }
    let Ok((mut threat, mut decision)) = actors.get_mut(impact.target) else {
        return;
    };
    let Ok(position) = positions.get(impact.target) else {
        return;
    };
    threat.target = impact.attacker;
    threat.position = impact
        .attacker
        .and_then(|target| positions.get(target).ok())
        .map_or(
            position.current.as_vec3() - impact.impulse.with_y(0.0).normalize_or(Vec3::X) * 3.0,
            |position| position.current.as_vec3(),
        );
    threat.expires = clock.0 + 100;
    decision.next_tick = clock.0;
}
