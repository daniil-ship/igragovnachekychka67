//! Процедурные текстуры: бетон стен, плитка пола, панели потолка, гало ламп.
//!
//! Генерируются в коде попиксельно (value-noise + пятна + швы), поэтому
//! asset-файлы не нужны - текстуры есть всегда, даже в пустой папке assets/.
//! У каждого акта свой облик ([`ActLook`]): дальше - темнее и кровавее.
//! Генератор детерминированный: лабиринт выглядит одинаково каждый запуск.

use bevy::prelude::*;
use bevy::render::render_asset::RenderAssetUsages;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::GameState;

/// Разрешение текстуры бетона (натягивается на грань стены 4x4 м).
const WALL_SIZE: u32 = 512;
/// Разрешение плитки пола (4x4 плитки на грань 4x4 м).
const FLOOR_SIZE: u32 = 256;
/// Разрешение панелей потолка.
const CEIL_SIZE: u32 = 256;

// ---------------------------------------------------------------------------
// Облик актов
// ---------------------------------------------------------------------------

/// Облик поверхностей для акта: свой сид шума, оттенок и кровь.
#[derive(Clone, Copy)]
pub struct ActLook {
    /// Сдвиг сидов шума (у каждого акта свой бетон).
    pub seed: i32,
    /// Оттенок поверхностей (множитель яркости).
    pub tint: [f32; 3],
    /// Сила красных разводов (кровь/ржавчина), 0.0 = чисто.
    pub rot: f32,
}

/// Облик поверхностей для акта: дальше - темнее и кровавее.
pub fn act_look(act: GameState) -> ActLook {
    match act {
        GameState::Act2_TheInsanity => ActLook {
            seed: 1000,
            tint: [0.9, 0.8, 0.8],
            rot: 0.4,
        },
        GameState::Act3_TheReactor => ActLook {
            seed: 2000,
            tint: [0.74, 0.66, 0.62],
            rot: 0.7,
        },
        _ => ActLook {
            seed: 0,
            tint: [1.0, 1.0, 1.0],
            rot: 0.0,
        },
    }
}

// ---------------------------------------------------------------------------
// Публичные строители текстур
// ---------------------------------------------------------------------------

/// Бетон стен: разводы + зерно + потёки + тёмные пятна + грязь книзу.
/// В актах 2-3 добавляются красные разводы крови и ржавчины.
pub fn build_wall_texture(look: ActLook) -> Image {
    const S: f32 = WALL_SIZE as f32;
    let seed = look.seed;
    new_rgba_image(WALL_SIZE, |x, y| {
        let fx = x as f32;
        let fy = y as f32;
        // Крупные разводы бетона и мелкое зерно.
        let blotches = fbm(fx / S * 6.0, fy / S * 6.0, 11 + seed);
        let grain = fbm(fx / S * 90.0, fy / S * 90.0, 23 + seed);
        let mut v = 0.52 + (blotches - 0.5) * 0.22 + (grain - 0.5) * 0.09;
        // Вертикальные потёки, усиливающиеся книзу.
        let streak = value_noise(fx / S * 40.0, 3.7, 37 + seed);
        v -= streak * streak * 0.08 * (fy / S);
        // Тёмные пятна грязи: 6 детерминированных кругов.
        for i in 0..6 {
            let cx = hash2(i, 7, 51 + seed) * S;
            let cy = hash2(i, 13, 52 + seed) * S;
            let r = (0.08 + hash2(i, 29, 53 + seed) * 0.14) * S;
            let dx = fx - cx;
            let dy = fy - cy;
            let d2 = (dx * dx + dy * dy) / (r * r);
            if d2 < 1.0 {
                v -= (1.0 - d2) * (0.10 + hash2(i, 41, 54 + seed) * 0.12);
            }
        }
        // Затемнение книзу (грязь у пола).
        v *= 1.0 - 0.22 * (fy / S) * (fy / S);
        let v = v.clamp(0.0, 1.0);
        // Кровь и ржавчина (акты 2-3): красные разводы поверх бетона.
        let rot =
            (fbm(fx / S * 4.0, fy / S * 7.0, 97 + seed) - 0.55).max(0.0) * 2.0 * look.rot;
        let t = look.tint;
        let r = (v * 0.96 * t[0] + rot * 0.38).clamp(0.0, 1.0);
        let g = (v * 0.97 * t[1] + rot * 0.05).clamp(0.0, 1.0);
        let b = (v * 1.03 * t[2]).clamp(0.0, 1.0);
        [
            (r * 255.0) as u8,
            (g * 255.0) as u8,
            (b * 255.0) as u8,
            255,
        ]
    })
}

/// Плитка пола 4x4: тёмные плиты, швы, разброс тона, грязный налёт.
/// В актах 2-3 - тёмные лужицы крови между плитами.
pub fn build_floor_texture(look: ActLook) -> Image {
    const S: f32 = FLOOR_SIZE as f32;
    const TILE: f32 = 64.0;
    let seed = look.seed;
    new_rgba_image(FLOOR_SIZE, |x, y| {
        let fx = x as f32;
        let fy = y as f32;
        let tx = (fx / TILE).floor() as i32;
        let ty = (fy / TILE).floor() as i32;
        let grout = fx % TILE < 3.0 || fy % TILE < 3.0;
        let mut v = if grout {
            0.055 // швы между плитами
        } else {
            0.14 + hash2(tx, ty, 71 + seed) * 0.05
                + (fbm(fx / S * 24.0, fy / S * 24.0, 72 + seed) - 0.5) * 0.05
        };
        v = v.clamp(0.0, 1.0);
        // Лужицы крови (акты 2-3).
        let rot =
            (fbm(fx / S * 3.0, fy / S * 3.0, 79 + seed) - 0.6).max(0.0) * 2.5 * look.rot;
        let t = look.tint;
        let r = (v * 0.92 * t[0] + rot * 0.30).clamp(0.0, 1.0);
        let g = (v * 0.95 * t[1] + rot * 0.03).clamp(0.0, 1.0);
        let b = (v * 1.05 * t[2]).clamp(0.0, 1.0);
        [
            (r * 255.0) as u8,
            (g * 255.0) as u8,
            (b * 255.0) as u8,
            255,
        ]
    })
}

/// Панели потолка 2x2: швы, пыльные плиты, пара потёков.
/// В актах 2-3 - ржавые разводы поверх панелей.
pub fn build_ceiling_texture(look: ActLook) -> Image {
    const S: f32 = CEIL_SIZE as f32;
    const PANEL: f32 = 128.0;
    let seed = look.seed;
    new_rgba_image(CEIL_SIZE, |x, y| {
        let fx = x as f32;
        let fy = y as f32;
        let seam = fx % PANEL < 2.0 || fy % PANEL < 2.0;
        let mut v = if seam {
            0.04
        } else {
            0.09 + (fbm(fx / S * 10.0, fy / S * 10.0, 81 + seed) - 0.5) * 0.04
        };
        for i in 0..3 {
            let cx = hash2(i, 3, 82 + seed) * S;
            let cy = hash2(i, 5, 83 + seed) * S;
            let r = (0.10 + hash2(i, 9, 84 + seed) * 0.12) * S;
            let dx = fx - cx;
            let dy = fy - cy;
            let d2 = (dx * dx + dy * dy) / (r * r);
            if d2 < 1.0 {
                v -= (1.0 - d2) * 0.05;
            }
        }
        v = v.clamp(0.0, 1.0);
        // Ржавые разводы (акты 2-3).
        let rot =
            (fbm(fx / S * 5.0, fy / S * 5.0, 89 + seed) - 0.62).max(0.0) * 2.0 * look.rot;
        let t = look.tint;
        let r = (v * t[0] + rot * 0.22).clamp(0.0, 1.0);
        let g = (v * t[1] + rot * 0.03).clamp(0.0, 1.0);
        let b = (v * 1.04 * t[2]).clamp(0.0, 1.0);
        [
            (r * 255.0) as u8,
            (g * 255.0) as u8,
            (b * 255.0) as u8,
            255,
        ]
    })
}

// ---------------------------------------------------------------------------
// Гало ламп
// ---------------------------------------------------------------------------

/// Мягкое круглое гало 128x128 для ламп (белое, тонируется материалом).
/// Используется с аддитивным блендингом, поэтому края полностью гаснут.
pub fn build_glow_texture() -> Image {
    new_rgba_image(128, |x, y| {
        let dx = (x as f32 / 127.0 - 0.5) * 2.0;
        let dy = (y as f32 / 127.0 - 0.5) * 2.0;
        let d = (dx * dx + dy * dy).sqrt().min(1.0);
        let core = 1.0 - smooth(d);
        let a = (core * core * 255.0) as u8;
        [255, 255, 255, a]
    })
}

// ---------------------------------------------------------------------------
// Каркас текстуры и value-noise
// ---------------------------------------------------------------------------

/// Собрать RGBA-текстуру `size`x`size` попиксельным генератором.
fn new_rgba_image(size: u32, pixels: impl Fn(u32, u32) -> [u8; 4]) -> Image {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            data.extend_from_slice(&pixels(x, y));
        }
    }
    Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// Детерминированный хэш координат в диапазоне [0, 1).
fn hash2(x: i32, y: i32, seed: i32) -> f32 {
    let mut h = x.wrapping_mul(374_761_393)
        ^ y.wrapping_mul(668_265_263)
        ^ seed.wrapping_mul(974_634_211);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    h ^= h >> 16;
    (h as u32 as f32) / (u32::MAX as f32)
}

/// Сглаживающий полином для интерполяции шума.
fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Value-noise с билинейной интерполяцией, диапазон примерно [0, 1].
fn value_noise(x: f32, y: f32, seed: i32) -> f32 {
    let xi = x.floor() as i32;
    let yi = y.floor() as i32;
    let xf = smooth(x - xi as f32);
    let yf = smooth(y - yi as f32);
    let a = hash2(xi, yi, seed);
    let b = hash2(xi + 1, yi, seed);
    let c = hash2(xi, yi + 1, seed);
    let d = hash2(xi + 1, yi + 1, seed);
    a + (b - a) * xf + (c - a) * yf + (a - b - c + d) * xf * yf
}

/// Фрактальный шум (4 октавы), диапазон примерно [0, 1].
fn fbm(x: f32, y: f32, seed: i32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 0.5;
    let mut freq = 1.0;
    for _ in 0..4 {
        sum += amp * value_noise(x * freq, y * freq, seed);
        freq *= 2.03;
        amp *= 0.5;
    }
    sum / 0.9375
}
