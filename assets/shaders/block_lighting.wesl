// UV1 carries independent, normalized sky and block light. A small ambient floor
// keeps unlit caves readable; block light is independent of the sun and shadows.
fn sky_visibility(light: vec2<f32>) -> f32 {
    return 0.025 + 0.975 * pow(clamp(light.x, 0.0, 1.0), 2.2);
}

fn block_irradiance(light: vec2<f32>) -> vec3<f32> {
    return vec3<f32>(1.0, 0.80, 0.60)
        * (1800.0 * pow(clamp(light.y, 0.0, 1.0), 2.2));
}
