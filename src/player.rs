//! Игрок: FPS-контроллер, стамина, принудительный бег, фонарик и HUD.
//!
//! Ключевая механика модуля — [`stamina_system`]: бег (Shift) расходует
//! стамину, а бег на пустой стамине включает режим [`ForcedRun`]
//! («принудительный бег»), который сводит героя с ума
//! (см. [`crate::hallucinations`]).

use bevy::input::mouse::MouseMotion;
use bevy::prelude::*;

use crate::hallucinations::Insanity;
use crate::{GameProgress, LevelColliders};

// ---------------------------------------------------------------------------
// Константы баланса
// ---------------------------------------------------------------------------

/// Скорость ходьбы, м/с.
const WALK_SPEED: f32 = 3.2;
/// Скорость бега, м/с (сохраняется и в принудительном беге — адреналин).
const RUN_SPEED: f32 = 6.0;
/// Чувствительность мыши.
const MOUSE_SENSITIVITY: f32 = 0.0028;
/// Высота глаз игрока над полом.
pub const EYE_HEIGHT: f32 = 1.65;
/// Радиус тела игрока для коллизий со стенами.
const PLAYER_RADIUS: f32 = 0.35;
/// Базовая яркость фонарика (канделы, физические единицы Bevy).
pub const FLASHLIGHT_INTENSITY: f32 = 900.0;

// ---------------------------------------------------------------------------
// Компоненты
// ---------------------------------------------------------------------------

/// Игрок (висит на камере). Хранит углы обзора от мыши.
#[derive(Component)]
pub struct Player {
    /// Рысканье (поворот вокруг Y).
    pub yaw: f32,
    /// Тангаж (наклон вверх/вниз).
    pub pitch: f32,
}

/// Шкала стамины (выносливости).
#[derive(Component)]
pub struct Stamina {
    /// Текущее значение от 0.0 до [`Stamina::MAX`].
    pub current: f32,
}

impl Stamina {
    /// Максимум шкалы.
    pub const MAX: f32 = 100.0;
    /// Расход стамины в секунду при обычном беге.
    const DRAIN_PER_SEC: f32 = 16.0;
    /// Восстановление в секунду на ходу.
    const REGEN_WALK_PER_SEC: f32 = 10.0;
    /// Восстановление в секунду стоя на месте.
    const REGEN_IDLE_PER_SEC: f32 = 18.0;
}

impl Default for Stamina {
    fn default() -> Self {
        Self { current: Self::MAX }
    }
}

/// Маркер принудительного бега: `true`, пока игрок бежит на пустой стамине.
///
/// Пока флаг поднят, система безумия копит скрытый параметр [`Insanity`].
#[derive(Component)]
pub struct ForcedRun(pub bool);

/// Фонарик (висит на [`SpotLight`](bevy::pbr::SpotLight), ребёнке камеры).
#[derive(Component)]
pub struct Flashlight {
    /// Базовая яркость, вокруг которой играет мерцание.
    pub base_intensity: f32,
}

// --- Маркеры элементов HUD (обновляются в [`hud_system`]) ---

/// Заливка полоски стамины (меняем ширину и цвет).
#[derive(Component)]
pub struct StaminaFill;

/// Предупреждение о принудительном беге.
#[derive(Component)]
pub struct WarningText;

/// Счётчик собранных фрагментов эха.
#[derive(Component)]
pub struct FragmentCounterText;

/// Красная виньетка безумия на весь экран.
#[derive(Component)]
pub struct InsanityVignette;

// ---------------------------------------------------------------------------
// Плагин
// ---------------------------------------------------------------------------

/// Регистрирует все системы игрока. Они работают только тогда, когда
/// разрешает [`crate::game_input_allowed`]: состояние `InGame`, нет активного
/// скримера и игра ещё не выиграна (тем самым на секунду скримера управление
/// полностью блокируется).
pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                mouse_look,
                player_movement,
                stamina_system,
                flashlight_flicker,
                hud_system,
            )
                .run_if(crate::game_input_allowed),
        );
    }
}

// ---------------------------------------------------------------------------
// Системы
// ---------------------------------------------------------------------------

/// Обзор мышью: рысканье/тангаж камеры с ограничением наклона.
fn mouse_look(
    mut motion: EventReader<MouseMotion>,
    mut query: Query<(&mut Player, &mut Transform)>,
) {
    let mut delta = Vec2::ZERO;
    for event in motion.read() {
        delta += event.delta;
    }
    if delta == Vec2::ZERO {
        return;
    }
    for (mut player, mut transform) in &mut query {
        player.yaw -= delta.x * MOUSE_SENSITIVITY;
        player.pitch = (player.pitch - delta.y * MOUSE_SENSITIVITY).clamp(-1.45, 1.45);
        transform.rotation = Quat::from_euler(EulerRot::YXZ, player.yaw, player.pitch, 0.0);
    }
}

/// Движение WASD/стрелки + Shift (бег) с простыми AABB-коллизиями стен.
fn player_movement(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut query: Query<(&Player, &mut Transform)>,
    colliders: Res<LevelColliders>,
) {
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);

    let mut input = Vec2::ZERO;
    if keys.pressed(KeyCode::KeyW) || keys.pressed(KeyCode::ArrowUp) {
        input.y += 1.0;
    }
    if keys.pressed(KeyCode::KeyS) || keys.pressed(KeyCode::ArrowDown) {
        input.y -= 1.0;
    }
    if keys.pressed(KeyCode::KeyA) || keys.pressed(KeyCode::ArrowLeft) {
        input.x -= 1.0;
    }
    if keys.pressed(KeyCode::KeyD) || keys.pressed(KeyCode::ArrowRight) {
        input.x += 1.0;
    }
    if input == Vec2::ZERO {
        return;
    }
    let input = input.normalize();

    // Скорость бега сохраняется, даже если стамина пуста:
    // герой бежит «из последних сил» (но безумие растёт, см. stamina_system).
    let speed = if shift { RUN_SPEED } else { WALK_SPEED };

    for (player, mut transform) in &mut query {
        let (sin, cos) = player.yaw.sin_cos();
        // Базис камеры в плоскости XZ.
        let forward = Vec3::new(-sin, 0.0, -cos);
        let right = Vec3::new(cos, 0.0, -sin);
        let wish = (forward * input.y + right * input.x) * speed * time.delta_secs();
        move_with_collision(&mut transform.translation, wish, &colliders.walls);
        transform.translation.y = EYE_HEIGHT;
    }
}

/// Сдвиг позиции с выталкиванием из AABB стен (по осям X и Z раздельно,
/// чтобы можно было скользить вдоль стен).
fn move_with_collision(
    position: &mut Vec3,
    delta: Vec3,
    walls: &[crate::WallAabb],
) {
    // Сначала X...
    position.x += delta.x;
    for wall in walls {
        let overlap_z =
            position.z + PLAYER_RADIUS > wall.min_z && position.z - PLAYER_RADIUS < wall.max_z;
        let overlap_x =
            position.x + PLAYER_RADIUS > wall.min_x && position.x - PLAYER_RADIUS < wall.max_x;
        if overlap_x && overlap_z {
            if delta.x > 0.0 {
                position.x = wall.min_x - PLAYER_RADIUS;
            } else if delta.x < 0.0 {
                position.x = wall.max_x + PLAYER_RADIUS;
            }
        }
    }
    // ...затем Z.
    position.z += delta.z;
    for wall in walls {
        let overlap_z =
            position.z + PLAYER_RADIUS > wall.min_z && position.z - PLAYER_RADIUS < wall.max_z;
        let overlap_x =
            position.x + PLAYER_RADIUS > wall.min_x && position.x - PLAYER_RADIUS < wall.max_x;
        if overlap_x && overlap_z {
            if delta.z > 0.0 {
                position.z = wall.min_z - PLAYER_RADIUS;
            } else if delta.z < 0.0 {
                position.z = wall.max_z + PLAYER_RADIUS;
            }
        }
    }
}

/// Стамина и принудительный бег.
///
/// - Обычный бег (`Shift` + движение) тратит [`Stamina::DRAIN_PER_SEC`]/с.
/// - Без бега стамина восстанавливается (стоя — быстрее).
/// - Если стамина `<= 0.0`, но игрок **продолжает** держать `Shift` и
///   двигаться, поднимается флаг [`ForcedRun`] — принудительный бег.
///   Скорость при этом не падает, но безумие начинает расти.
fn stamina_system(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut query: Query<(&mut Stamina, &mut ForcedRun)>,
) {
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let moving = keys.pressed(KeyCode::KeyW)
        || keys.pressed(KeyCode::KeyA)
        || keys.pressed(KeyCode::KeyS)
        || keys.pressed(KeyCode::KeyD)
        || keys.pressed(KeyCode::ArrowUp)
        || keys.pressed(KeyCode::ArrowLeft)
        || keys.pressed(KeyCode::ArrowDown)
        || keys.pressed(KeyCode::ArrowRight);

    let wants_run = shift && moving;
    let dt = time.delta_secs();

    for (mut stamina, mut forced) in &mut query {
        if wants_run && stamina.current > 0.0 {
            stamina.current = (stamina.current - Stamina::DRAIN_PER_SEC * dt).max(0.0);
        } else if !wants_run {
            let rate = if moving {
                Stamina::REGEN_WALK_PER_SEC
            } else {
                Stamina::REGEN_IDLE_PER_SEC
            };
            stamina.current = (stamina.current + rate * dt).min(Stamina::MAX);
        }
        // Принудительный бег: Shift + движение при пустой стамине.
        forced.0 = wants_run && stamina.current <= 0.0;
    }
}

/// Мерцание фонарика. Лёгкое дрожание есть всегда, но чем выше скрытое
/// безумие — тем сильнее дёргается свет и тем чаще случаются провалы.
fn flashlight_flicker(
    time: Res<Time>,
    insanity_query: Query<&Insanity>,
    mut lamp_query: Query<(&Flashlight, &mut SpotLight)>,
) {
    let madness = insanity_query
        .iter()
        .next()
        .map(|insanity| insanity.0 / 100.0)
        .unwrap_or(0.0);

    let t = time.elapsed_secs();
    // Детерминированный псевдошум из суммы синусов — без аллокаций и RNG.
    let noise = (t * 37.0).sin() * 0.5 + (t * 23.0 + 1.7).sin() * 0.3 + (t * 61.0 + 4.2).sin() * 0.2;
    let amount = 0.04 + madness * 0.35;

    for (lamp, mut light) in &mut lamp_query {
        light.intensity = lamp.base_intensity * (1.0 + noise * amount);
        // Редкие глубокие провалы света при высоком безумии.
        if madness > 0.6 && (t * 3.1).sin() > 0.985 {
            light.intensity *= 0.25;
        }
    }
}

/// Обновление HUD: полоска стамины, предупреждение, счётчик и виньетка.
///
/// Обратите внимание на `Without`-фильтры: они нужны, чтобы запросы,
/// мутабельно трогающие одни и те же типы компонентов (`Text`,
/// `BackgroundColor`), были гарантированно непересекающимися.
fn hud_system(
    time: Res<Time>,
    player_query: Query<(&Stamina, &ForcedRun, &Insanity)>,
    mut fill_query: Query<
        (&mut Node, &mut BackgroundColor),
        (With<StaminaFill>, Without<InsanityVignette>),
    >,
    mut warning_query: Query<&mut Text, (With<WarningText>, Without<FragmentCounterText>)>,
    mut counter_query: Query<&mut Text, (With<FragmentCounterText>, Without<WarningText>)>,
    mut vignette_query: Query<&mut BackgroundColor, (With<InsanityVignette>, Without<StaminaFill>)>,
    progress: Res<GameProgress>,
) {
    let Some((stamina, forced, insanity)) = player_query.iter().next() else {
        return;
    };

    // Полоска стамины: ширина по проценту + цвет от зелёного к красному.
    let ratio = (stamina.current / Stamina::MAX).clamp(0.0, 1.0);
    for (mut node, mut color) in &mut fill_query {
        node.width = Val::Percent(ratio * 100.0);
        color.0 = Color::srgb(0.9 * (1.0 - ratio) + 0.15, 0.75 * ratio + 0.1, 0.12);
    }

    // Предупреждение видно только в принудительном беге.
    for mut text in &mut warning_query {
        let message = if forced.0 {
            "!! FORCED RUN — YOUR MIND IS SLIPPING !!"
        } else {
            ""
        };
        if text.0.as_str() != message {
            text.0 = message.to_string();
        }
    }

    // Счётчик фрагментов.
    for mut text in &mut counter_query {
        let message = format!("FRAGMENTS: {}/{}", progress.collected, progress.total);
        if text.0 != message {
            text.0 = message;
        }
    }

    // Виньетка безумия: красная дымка по краям, пульсирует.
    // Само безумие скрыто (числа нет), но его присутствие чувствуется.
    let pulse = 0.75 + 0.25 * (time.elapsed_secs() * 3.0).sin();
    let alpha = (insanity.0 / 100.0 * 0.30 * pulse).clamp(0.0, 0.35);
    for mut color in &mut vignette_query {
        color.0 = Color::srgba(0.55, 0.02, 0.03, alpha);
    }
}
