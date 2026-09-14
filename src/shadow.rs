//! Теневая сущность: 3D-скример, бросающийся на игрока.
//!
//! Половина срабатываний скримера идёт в 3D-режиме (см. [`fire_shadow`]:
//! вместо fullscreen-картинки прямо перед игроком появляется чёрная тварь,
//! собранная из примитивов (торс, вытянутая голова, длинные руки,
//! светящиеся глаза), и за 0.6 секунды бросается в лицо. Звук, тряска
//! камеры и блокировка управления - те же, что у обычного скримера.
//! Точка появления подбирается так, чтобы тварь не оказалась внутри стены.

use bevy::prelude::*;
use bevy::state::state_scoped::StateScoped;

use crate::hallucinations::ScreamerState;
use crate::{AppState, WallAabb};

/// Дальность появления тени перед игроком, метры.
const SHADOW_SPAWN_DIST: f32 = 3.5;
/// Дистанция, на которую тень бросается (прямо в лицо), метры.
const SHADOW_LUNGE_DIST: f32 = 0.7;
/// Ближе этой дистанции тень не появляется.
const SHADOW_MIN_DIST: f32 = 0.8;
/// Длительность рывка, секунды (дальше тварь висит перед лицом до конца секунды).
const LUNGE_TIME: f32 = 0.6;
/// Радиус тела для проверки "не в стене ли точка".
const BODY_RADIUS: f32 = 0.35;

// ---------------------------------------------------------------------------
// Компоненты и ресурсы
// ---------------------------------------------------------------------------

/// Маркер корня монстра (двигаем корень - дети едут следом).
#[derive(Component)]
struct ShadowRoot;

/// Маркер частей тела (нужен, чтобы надёжно удалить всё после секунды).
#[derive(Component)]
struct ShadowPart;

/// Состояние выпада тени.
#[derive(Resource, Default)]
pub struct ShadowLunge {
    /// Выпад активен (идёт секунда 3D-скримера).
    pub active: bool,
    /// Сколько секунд длится текущий выпад.
    elapsed: f32,
    /// Откуда бросается (точка появления).
    from: Vec3,
    /// Куда бросается (лицо игрока).
    to: Vec3,
}

// ---------------------------------------------------------------------------
// Плагин
// ---------------------------------------------------------------------------

/// Регистрирует тиканье выпада тени.
pub struct ShadowPlugin;

impl Plugin for ShadowPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ShadowLunge::default()).add_systems(
            Update,
            tick_shadow_lunge.run_if(in_state(AppState::InGame)),
        );
    }
}

// ---------------------------------------------------------------------------
// Спавн и тиканье
// ---------------------------------------------------------------------------

/// Спавн тени перед игроком. Вызывается из триггера скримера в 3D-режиме.
/// Возвращает управление сразу; движением занимается [`tick_shadow_lunge`].
pub fn spawn_shadow(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    lunge: &mut ShadowLunge,
    player_pos: Vec3,
    yaw: f32,
    walls: &[WallAabb],
) {
    let (sin, cos) = yaw.sin_cos();
    let forward = Vec3::new(-sin, 0.0, -cos);
    let right = Vec3::new(cos, 0.0, -sin);

    // Ищем свободную точку от 3.5 м до 0.8 м перед игроком.
    let mut dist = SHADOW_MIN_DIST;
    let mut t = SHADOW_SPAWN_DIST;
    while t >= SHADOW_MIN_DIST {
        if spot_free(player_pos + forward * t, walls) {
            dist = t;
            break;
        }
        t -= 0.15;
    }
    let from = Vec3::new(
        player_pos.x + forward.x * dist,
        0.0,
        player_pos.z + forward.z * dist,
    );
    let to = Vec3::new(
        player_pos.x + forward.x * SHADOW_LUNGE_DIST,
        0.0,
        player_pos.z + forward.z * SHADOW_LUNGE_DIST,
    );

    let skin = materials.add(StandardMaterial {
        base_color: Color::srgb(0.008, 0.008, 0.012),
        perceptual_roughness: 1.0,
        ..default()
    });
    let eye_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.06, 0.04),
        unlit: true,
        ..default()
    });

    // Тело ~2.5 м ростом: торс-капсула, вытянутая голова, длинные руки
    // и два светящихся глаза на стороне, обращённой к игроку.
    let to_player = -forward;
    commands
        .spawn((
            Transform::from_translation(from),
            ShadowRoot,
            StateScoped(AppState::InGame),
        ))
        .with_children(|body| {
            // Торс.
            body.spawn((
                Mesh3d(meshes.add(Capsule3d::new(0.28, 1.1))),
                MeshMaterial3d(skin.clone()),
                Transform::from_xyz(0.0, 1.25, 0.0),
                ShadowPart,
                StateScoped(AppState::InGame),
            ));
            // Вытянутая голова.
            let head = Vec3::new(0.0, 2.2, 0.0);
            body.spawn((
                Mesh3d(meshes.add(Sphere::new(0.22).mesh().uv(16, 12))),
                MeshMaterial3d(skin.clone()),
                Transform {
                    translation: head,
                    scale: Vec3::new(1.0, 1.5, 0.9),
                    ..default()
                },
                ShadowPart,
                StateScoped(AppState::InGame),
            ));
            // Глаза-огоньки.
            for side in [-1.0f32, 1.0] {
                let eye =
                    head + to_player * 0.20 + right * (0.085 * side) + Vec3::new(0.0, 0.08, 0.0);
                body.spawn((
                    Mesh3d(meshes.add(Sphere::new(0.045).mesh().uv(12, 8))),
                    MeshMaterial3d(eye_mat.clone()),
                    Transform::from_translation(eye),
                    ShadowPart,
                    StateScoped(AppState::InGame),
                ));
            }
            // Длинные руки с лёгким наклоном наружу.
            for side in [-1.0f32, 1.0] {
                body.spawn((
                    Mesh3d(meshes.add(Capsule3d::new(0.07, 0.95))),
                    MeshMaterial3d(skin.clone()),
                    Transform {
                        translation: Vec3::new(0.44 * side, 1.15, 0.0),
                        rotation: Quat::from_rotation_z(0.18 * -side),
                        ..default()
                    },
                    ShadowPart,
                    StateScoped(AppState::InGame),
                ));
            }
        });

    lunge.active = true;
    lunge.elapsed = 0.0;
    lunge.from = from;
    lunge.to = to;
}

/// Свободна ли точка: круг радиуса тела не пересекает ни одну стену.
fn spot_free(pos: Vec3, walls: &[WallAabb]) -> bool {
    for w in walls {
        if pos.x + BODY_RADIUS > w.min_x
            && pos.x - BODY_RADIUS < w.max_x
            && pos.z + BODY_RADIUS > w.min_z
            && pos.z - BODY_RADIUS < w.max_z
        {
            return false;
        }
    }
    true
}

/// Выпад тени: рывок к лицу с ускорением + нарастающее дрожание +
/// нависание (тварь растёт). Когда секунда скримера кончается -
/// тварь исчезает вместе с ней.
fn tick_shadow_lunge(
    time: Res<Time>,
    mut commands: Commands,
    screamer: Res<ScreamerState>,
    mut lunge: ResMut<ShadowLunge>,
    mut roots: Query<(Entity, &mut Transform), With<ShadowRoot>>,
    parts: Query<Entity, With<ShadowPart>>,
) {
    if !lunge.active {
        return;
    }
    if !screamer.active {
        for (entity, _) in &roots {
            commands.entity(entity).despawn();
        }
        for entity in &parts {
            commands.entity(entity).despawn();
        }
        lunge.active = false;
        return;
    }
    lunge.elapsed += time.delta_secs();
    let t = (lunge.elapsed / LUNGE_TIME).min(1.0);
    let ease = t * t; // ускорение к концу рывка
    // Дрожание нарастает к концу выпада.
    let shake = 0.03 * t;
    let sx = (lunge.elapsed * 91.0).sin() * shake;
    let sz = (lunge.elapsed * 113.0).cos() * shake;
    for (_, mut transform) in &mut roots {
        transform.translation = lunge.from.lerp(lunge.to, ease) + Vec3::new(sx, 0.0, sz);
        transform.scale = Vec3::splat(1.0 + ease * 0.25);
    }
}
