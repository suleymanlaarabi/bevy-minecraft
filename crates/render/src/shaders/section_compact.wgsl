// Gathers the visible section draws of one view into a single u32 index list, in
// draw order: index `out + i` holds the arena vertex of the job's index `first + i`.
// A job is `(first index, count | pass << 29, base vertex, out)`.
@group(0) @binding(0) var<storage, read> jobs: array<vec4<u32>>;
@group(0) @binding(1) var<storage, read_write> output: array<u32>;
// Pass index arenas: packed u16 terrain and shadow hull, u32 cutouts, model quads
// and translucent quads.
@group(0) @binding(2) var<storage, read> terrain: array<u32>;
@group(0) @binding(3) var<storage, read> cutout: array<u32>;
@group(0) @binding(4) var<storage, read> quads: array<u32>;
@group(0) @binding(5) var<storage, read> hull: array<u32>;
// Translucent quads, re-sorted in place as the eye moves.
@group(0) @binding(6) var<storage, read> translucent: array<u32>;

const LANES: u32 = 64u;

@compute @workgroup_size(64)
fn gather(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
    @builtin(local_invocation_index) lane: u32,
) {
    let index = group.x + group.y * groups.x;
    if index >= arrayLength(&jobs) {
        return;
    }
    let job = jobs[index];
    let count = job.y & 0x1fffffffu;
    let kind = job.y >> 29u;
    for (var i = lane; i < count; i += LANES) {
        let at = job.x + i;
        var vertex: u32;
        switch kind {
            case 0u: {
                vertex = (terrain[at >> 1u] >> ((at & 1u) * 16u)) & 0xffffu;
            }
            case 4u: {
                vertex = (hull[at >> 1u] >> ((at & 1u) * 16u)) & 0xffffu;
            }
            case 1u: {
                vertex = cutout[at];
            }
            case 5u: {
                vertex = translucent[at];
            }
            default: {
                vertex = quads[at];
            }
        }
        output[job.w + i] = job.z + vertex;
    }
}
