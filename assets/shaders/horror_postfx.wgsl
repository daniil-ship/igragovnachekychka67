// Хоррор-постобработка: зерно плёнки / VHS-шум + виньетка от безумия.
//
// Полноэкранный треугольник Bevy даёт готовые UV (см. пример
// `custom_post_processing`): вершинный шейдер импортируется, пишем фрагментный.

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;
struct HorrorPostFx {
    time: f32,
    insanity: f32,
    grain_on: f32,
    vignette_on: f32,
}
@group(0) @binding(2) var<uniform> settings: HorrorPostFx;

// Хэш-зерно: дрожит каждый кадр (время подмешано в координаты).
fn hash(uv: vec2<f32>, t: f32) -> f32 {
    let d = dot(uv + fract(vec2<f32>(t * 13.73, t * 7.31)), vec2<f32>(12.9898, 78.233));
    return fract(sin(d) * 43758.5453);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    var color = textureSample(screen_texture, texture_sampler, in.uv);
    // Зерно плёнки / VHS-шум: есть всегда, с безумием усиливается.
    if (settings.grain_on > 0.5) {
        let g = hash(in.uv * vec2<f32>(1920.0, 1080.0), settings.time) - 0.5;
        let amount = 0.045 + settings.insanity * 0.075;
        color = vec4<f32>(color.rgb + vec3<f32>(g * amount), color.a);
    }
    // Виньетка: лёгкая всегда, с безумием сгущается и дышит.
    if (settings.vignette_on > 0.5) {
        let d = distance(in.uv, vec2<f32>(0.5, 0.5));
        let pulse = 0.92 + 0.08 * sin(settings.time * 2.6);
        let strength = (0.35 + settings.insanity * 0.85) * pulse;
        let v = 1.0 - smoothstep(0.3, 0.9, d * strength);
        color = vec4<f32>(color.rgb * v, color.a);
    }
    return color;
}
