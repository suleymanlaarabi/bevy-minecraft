use super::CarverOutput;
use super::{Carver, overworld_carve_state, place_carved_block};
use pumpkin_data::carver::{CarverAdditionalConfig, CarverConfig, HeightProvider};
use pumpkin_util::math::vector2::Vector2;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::{RandomGenerator, RandomImpl};
use std::f32::consts::PI;

pub struct CaveCarver;

impl Carver for CaveCarver {
    fn carve(
        &self,
        config: &CarverConfig,
        output: &mut dyn CarverOutput,
        random: &mut RandomGenerator,
        chunk_pos: &Vector2<i32>,
        carver_chunk_pos: &Vector2<i32>,
        min_gen_y: i8,
        gen_depth: u16,
        legacy_random_source: bool,
    ) {
        let CarverAdditionalConfig::Cave(ref cave_config) = config.additional else {
            return;
        };

        let max_distance = (4 * 2 - 1) << 4;
        let cave_count = cave_config.count.get(random);

        for _ in 0..cave_count {
            let x = (carver_chunk_pos.x << 4) + random.next_bounded_i32(16);
            let y = get_height(&config.y, random, min_gen_y, gen_depth) as f64;
            let z = (carver_chunk_pos.y << 4) + random.next_bounded_i32(16);

            let horizontal_radius_multiplier =
                cave_config.horizontal_radius_multiplier.get(random) as f64;
            let vertical_radius_multiplier =
                cave_config.vertical_radius_multiplier.get(random) as f64;
            let start_vertical_radius_multiplier =
                cave_config.start_vertical_radius_multiplier.get(random) as f64;
            let floor_level = cave_config.floor_level.get(random) as f64;

            let tunnels = if random.next_bounded_i32(4) == 0 {
                let room_vertical_radius_multiplier =
                    cave_config.room_vertical_radius_multiplier.get(random) as f64;
                let thickness = 1.0 + random.next_f32() * 6.0;
                Self::create_room(
                    *chunk_pos,
                    output,
                    x as f64,
                    y,
                    z as f64,
                    thickness,
                    room_vertical_radius_multiplier,
                    floor_level,
                );
                1 + random.next_bounded_i32(4)
            } else {
                1
            };

            for _ in 0..tunnels {
                let horizontal_rotation = random.next_f32() * PI * 2.0;
                let vertical_rotation = (random.next_f32() - 0.5) / 4.0;
                let mut thickness = cave_config.thickness.get(random);
                if cave_config.weird_thickness_bias && random.next_bounded_i32(10) == 0 {
                    thickness *= random.next_f32() * random.next_f32() * 3.0 + 1.0;
                }
                let distance = max_distance - random.next_bounded_i32(max_distance / 4);
                let tunnel_seed = random.next_i64();

                Self::create_tunnel(
                    *chunk_pos,
                    output,
                    tunnel_seed,
                    x as f64,
                    y,
                    z as f64,
                    horizontal_radius_multiplier,
                    vertical_radius_multiplier,
                    thickness,
                    horizontal_rotation,
                    vertical_rotation,
                    0,
                    distance,
                    start_vertical_radius_multiplier,
                    floor_level,
                    legacy_random_source,
                );
            }
        }
    }
}

impl CaveCarver {
    #[allow(clippy::too_many_arguments)]
    fn create_room(
        chunk_pos: Vector2<i32>,
        output: &mut dyn CarverOutput,
        x: f64,
        y: f64,
        z: f64,
        thickness: f32,
        y_scale: f64,
        floor_level: f64,
    ) {
        let horizontal_radius =
            1.5 + f64::from(pumpkin_util::math::sin(std::f32::consts::FRAC_PI_2) * thickness);
        let vertical_radius = horizontal_radius * y_scale;
        super::carve_ellipsoid(
            &chunk_pos,
            x + 1.0,
            y,
            z,
            horizontal_radius,
            vertical_radius,
            output,
            |xd, yd, zd, _world_y| Self::should_skip(xd, yd, zd, floor_level),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn create_tunnel(
        chunk_pos: Vector2<i32>,
        output: &mut dyn CarverOutput,
        tunnel_seed: i64,
        mut x: f64,
        mut y: f64,
        mut z: f64,
        horizontal_radius_multiplier: f64,
        vertical_radius_multiplier: f64,
        thickness: f32,
        mut horizontal_rotation: f32,
        mut vertical_rotation: f32,
        step: i32,
        dist: i32,
        y_scale: f64,
        floor_level: f64,
        legacy_random_source: bool,
    ) {
        let mut random = super::new_carver_random(tunnel_seed as u64, legacy_random_source);
        let split_point = random.next_bounded_i32(dist / 2) + dist / 4;
        let is_steep = random.next_bounded_i32(6) == 0;
        let mut y_rota = 0.0f32;
        let mut x_rota = 0.0f32;

        for current_step in step..dist {
            let progress_arg = PI * current_step as f32 / dist as f32;
            let horizontal_radius =
                1.5 + f64::from(pumpkin_util::math::sin(progress_arg) * thickness);
            let vertical_radius = horizontal_radius * y_scale;
            let cos_x = pumpkin_util::math::cos(vertical_rotation);
            x += f64::from(pumpkin_util::math::cos(horizontal_rotation) * cos_x);
            y += f64::from(pumpkin_util::math::sin(vertical_rotation));
            z += f64::from(pumpkin_util::math::sin(horizontal_rotation) * cos_x);

            vertical_rotation =
                (vertical_rotation * if is_steep { 0.92 } else { 0.7 }) + (x_rota * 0.1);
            horizontal_rotation += y_rota * 0.1;
            x_rota = (x_rota * 0.9)
                + ((random.next_f32() - random.next_f32()) * random.next_f32() * 2.0);
            y_rota = (y_rota * 0.75)
                + ((random.next_f32() - random.next_f32()) * random.next_f32() * 4.0);

            if current_step == split_point && thickness > 1.0 {
                Self::create_tunnel(
                    chunk_pos,
                    output,
                    random.next_i64(),
                    x,
                    y,
                    z,
                    horizontal_radius_multiplier,
                    vertical_radius_multiplier,
                    random.next_f32() * 0.5 + 0.5,
                    horizontal_rotation - (PI / 2.0),
                    vertical_rotation / 3.0,
                    current_step,
                    dist,
                    1.0,
                    floor_level,
                    legacy_random_source,
                );
                Self::create_tunnel(
                    chunk_pos,
                    output,
                    random.next_i64(),
                    x,
                    y,
                    z,
                    horizontal_radius_multiplier,
                    vertical_radius_multiplier,
                    random.next_f32() * 0.5 + 0.5,
                    horizontal_rotation + (PI / 2.0),
                    vertical_rotation / 3.0,
                    current_step,
                    dist,
                    1.0,
                    floor_level,
                    legacy_random_source,
                );
                return;
            }

            if random.next_bounded_i32(4) != 0 {
                if !super::can_reach(&chunk_pos, x, z, current_step, dist, thickness) {
                    return;
                }

                super::carve_ellipsoid(
                    &chunk_pos,
                    x,
                    y,
                    z,
                    horizontal_radius * horizontal_radius_multiplier,
                    vertical_radius * vertical_radius_multiplier,
                    output,
                    |xd, yd, zd, _world_y| Self::should_skip(xd, yd, zd, floor_level),
                );
            }
        }
    }

    fn should_skip(xd: f64, yd: f64, zd: f64, floor_level: f64) -> bool {
        yd <= floor_level || xd * xd + yd * yd + zd * zd >= 1.0
    }

    pub fn carve_block(
        run: &mut super::CarveRun,
        _config: &CarverConfig,
        x: i32,
        y: i32,
        z: i32,
        has_grass: &mut bool,
    ) -> bool {
        let current_state = run.chunk.get_block_state(&Vector3::new(x, y, z));
        if current_state == pumpkin_data::Block::BEDROCK.default_state.id {
            return false;
        }
        let block = pumpkin_data::Block::from_state_id(current_state);

        if block.id == pumpkin_data::Block::GRASS_BLOCK.id
            || block.id == pumpkin_data::Block::MYCELIUM.id
        {
            *has_grass = true;
        }

        let Some((state, should_schedule_fluid_update)) = overworld_carve_state(run, x, y, z)
        else {
            return false;
        };

        let overworld = run.ctx.carver_aquifer.is_some();

        place_carved_block(
            run,
            Vector3::new(x, y, z),
            state,
            should_schedule_fluid_update,
            *has_grass,
            overworld,
        );

        true
    }
}

pub fn get_height(p: &HeightProvider, random: &mut RandomGenerator, min_y: i8, height: u16) -> i32 {
    match p {
        HeightProvider::Uniform(p) => {
            let min = p.min_inclusive.get_y(min_y as i16, height);
            let max = p.max_inclusive.get_y(min_y as i16, height);
            if min > max {
                min
            } else {
                random.next_inbetween_i32(min, max)
            }
        }
        HeightProvider::Trapezoid(p) => {
            let i = p.min_inclusive.get_y(min_y as i16, height);
            let j = p.max_inclusive.get_y(min_y as i16, height);
            if i > j {
                return i;
            }
            let plateau = p.plateau.unwrap_or(0);
            let k = j - i;
            if plateau >= k {
                random.next_inbetween_i32(i, j)
            } else {
                let l = (k - plateau) / 2;
                let m = k - l;
                i + random.next_inbetween_i32(0, m) + random.next_inbetween_i32(0, l)
            }
        }
        HeightProvider::VeryBiasedToBottom(p) => {
            let min = p.min_inclusive.get_y(min_y as i16, height);
            let max = p.max_inclusive.get_y(min_y as i16, height);
            let inner = p.inner.map_or(1, std::num::NonZero::get) as i32;
            if max - min - inner < 0 {
                min
            } else {
                let upper_inclusive = random.next_inbetween_i32(min + inner, max);
                let biased_upper_inclusive = random.next_inbetween_i32(min, upper_inclusive - 1);
                random.next_inbetween_i32(min, biased_upper_inclusive - 1 + inner)
            }
        }
    }
}

