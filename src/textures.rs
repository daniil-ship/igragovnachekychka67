//! Процедурные текстуры: бетон стен, плитка пола, панели потолка,
//! виньетка, гало ламп и зерно плёнки.
//!
//! Генерируются в коде попиксельно (value-noise + пятна + швы), поэтому
//! asset-файлы не нужны - текстуры есть всегда, даже в пустой папке assets/.
//! Генератор детерминированный: лабиринт выглядит одинаково каждый запуск.

use bevy::prelude::*;
use bevy::render::render_asset::RenderAssetUsages;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

/// Разрешение текстуры бетона (натягивается на грань стены 4x4 м).
const WALL_SIZE: u32 = 512;
/// Разрешение плитки пола (4x4 плитки на грань 4x4 м).
const FLOOR_SIZE: u32 = 256;
/// Разрешение панелей потолка.
const CEIL_SIZE: u32 = 256;

// ---------------------------------------------------------------------------
// Публичные строители текстур
// ---------------------------------------------------------------------------

/// Бетон стен: разводы + зерно + потёки + тёмные пятна + грязь книзу.
pub fn build_wall_texture() -> Image {
    const S: f32 = WALL_SIZE as f32;
    new_rgba_image(WALL_SIZE, |x, y| {
        let fx = x as f32;
        let fy = y as f32;
        // Крупные разводы бетона и мелкое зерно.
        let blotches = fbm(fx / S * 6.0, fy / S * 6.0, 11);
        let grain = fbm(fx / S * 90.0, fy / S * 90.0, 23);
        let mut v = 0.52 + (blotches - 0.5) * 0.22 + (grain - 0.5) * 0.09;
        // Вертикальные потёки, усиливающиеся книзу.
        let streak = value_noise(fx / S * 40.0, 3.7, 37);
        v -= streak * streak * 0.08 * (fy / S);
        // Тёмные пятна грязи: 6 детерминированных кругов.
        for i in 0..6 {
            let cx = hash2(i, 7, 51) * S;
            let cy = hash2(i, 13, 52) * S;
            let r = (0.08 + hash2(i, 29, 53) * 0.14) * S;
            let dx = fx - cx;
            let dy = fy - cy;
            let d2 = (dx * dx + dy * dy) / (r * r);
            if d2 < 1.0 {
                v -= (1.0 - d2) * (0.10 + hash2(i, 41, 54) * 0.12);
            }
        }
        // Затемнение книзу (грязь у пола).
        v *= 1.0 - 0.22 * (fy / S) * (fy / S);
        let v = v.clamp(0.0, 1.0);
        // Холодный оттенок бетона.
        [
            (v * 0.96 * 255.0) as u8,
            (v * 0.97 * 255.0) as u8,
            (v * 1.03 * 255.0).min(255.0) as u8,
            255,
        ]
    })
}

/// Плитка пола 4x4: тёмные плиты, швы, разброс тона, грязный налёт.
pub fn build_floor_texture() -> Image {
    const S: f32 = FLOOR_SIZE as f32;
    const TILE: f32 = 64.0;
    new_rgba_image(FLOOR_SIZE, |x, y| {
        let fx = x as f32;
        let fy = y as f32;
        let tx = (fx / TILE).floor() as i32;
        let ty = (fy / TILE).floor() as i32;
        let grout = fx % TILE < 3.0 || fy % TILE < 3.0;
        let mut v = if grout {
            0.055 // швы между плитами
        } else {
            0.14 + hash2(tx, ty, 71) * 0.05
                + (fbm(fx / S * 24.0, fy / S * 24.0, 72) - 0.5) * 0.05
        };
        v = v.clamp(0.0, 1.0);
        [
            (v * 0.92 * 255.0) as u8,
            (v * 0.95 * 255.0) as u8,
            (v * 1.05 * 255.0).min(255.0) as u8,
            255,
        ]
    })
}

/// Панели потолка 2x2: швы, пыльные плиты, пара потёков.
pub fn build_ceiling_texture() -> Image {
    const S: f32 = CEIL_SIZE as f32;
    const PANEL: f32 = 128.0;
    new_rgba_image(CEIL_SIZE, |x, y| {
        let fx = x as f32;
        let fy = y as f32;
        let seam = fx % PANEL < 2.0 || fy % PANEL < 2.0;
        let mut v = if seam {
            0.04
        } else {
            0.09 + (fbm(fx / S * 10.0, fy / S * 10.0, 81) - 0.5) * 0.04
        };
        for i in 0..3 {
            let cx = hash2(i, 3, 82) * S;
            let cy = hash2(i, 5, 83) * S;
            let r = (0.10 + hash2(i, 9, 84) * 0.12) * S;
            let dx = fx - cx;
            let dy = fy - cy;
            let d2 = (dx * dx + dy * dy) / (r * r);
            if d2 < 1.0 {
                v -= (1.0 - d2) * 0.05;
            }
        }
        v = v.clamp(0.0, 1.0);
        [
            (v * 255.0) as u8,
            (v * 255.0) as u8,
            (v * 1.04 * 255.0).min(255.0) as u8,
            255,
        ]
    })
}

// ---------------------------------------------------------------------------
// Кино-оверлеи: виньетка, гало ламп, зерно плёнки
// ---------------------------------------------------------------------------

/// Тёмная виньетка 256x256: прозрачный центр, затемнение к краям.
/// Кладётся UI-нодой на весь экран поверх 3D-картинки.
pub fn build_vignette_texture() -> Image {
    new_rgba_image(256, |x, y| {
        let dx = (x as f32 / 255.0 - 0.5) * 2.0;
        let dy = (y as f32 / 255.0 - 0.5) * 2.0;
        let d = (dx * dx + dy * dy).sqrt();
        let t = ((d - 0.55) / 0.8).clamp(0.0, 1.0);
        let a = (smooth(t) * 0.68 * 255.0) as u8;
        [0, 0, 0, a]
    })
}

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

/// Один кадр зерна плёнки 128x128 (`seed` 0..3): случайная серая пыль.
/// Кадры переключаются ~11 раз в секунду поверх всего HUD.
pub fn build_grain_frame(seed: i32) -> Image {
    new_rgba_image(128, |x, y| {
        let v = (hash2(x as i32, y as i32, 91 + seed * 101) * 255.0) as u8;
        [v, v, v, 13]
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
