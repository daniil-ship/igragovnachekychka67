//! Теневая сущность: 3D-скример, бросающийся на игрока.
//!
//! Половина срабатываний скримера идёт в 3D-режиме (см. [`fire_shadow`]:
//! вместо fullscreen-картинки прямо перед игроком появляется чёрная тварь,
//! собранная из примитивов (торс, вытянутая голова, длинные руки,
//! светящиеся глаза), и за 0.6 секунды бросается в лицо. Звук, тряска
//! камеры и блокировка управления - те же, что у обычного скримера.
//! Точка появления подбирается так, чтобы тварь не оказалась внутри стены.
//!
//! Вторая роль модуля - Тень Отречения ([`MonsterFade`]): постоянный
//! монстр-сталкер Актов 2-3. Патрулирует коридоры, смотрит на игрока,
//! в Акте 3 охотится, а под прямым лучом фонаря отступает. Поимка
//! вскипает безумие до порога (= мгновенный скример).

use bevy::prelude::*;
use bevy::state::state_scoped::StateScoped;
use rand::Rng;

use crate::hallucinations::{Insanity, ScreamerState};
use crate::player::{Flashlight, Player, move_with_collision};
use crate::{GameState, LevelColliders, WallAabb};

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
            (tick_shadow_lunge, monster_ai).run_if(crate::in_act),
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
    act: GameState,
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
            StateScoped(act),
        ))
        .with_children(|body| {
            // Торс.
            body.spawn((
                Mesh3d(meshes.add(Capsule3d::new(0.28, 1.1))),
                MeshMaterial3d(skin.clone()),
                Transform::from_xyz(0.0, 1.25, 0.0),
                ShadowPart,
                StateScoped(act),
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
                StateScoped(act),
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
                    StateScoped(act),
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
                    StateScoped(act),
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

// ---------------------------------------------------------------------------
// Тень Отречения: постоянный монстр-сталкер (Акты 2-3)
// ---------------------------------------------------------------------------

/// Скорость патрулирования, м/с.
const MONSTER_PATROL_SPEED: f32 = 1.2;
/// Скорость погони за игроком, м/с (пешком не уйти, бегом - да).
const MONSTER_CHASE_SPEED: f32 = 3.0;
/// Скорость отступления под лучом фонаря, м/с.
const MONSTER_RETREAT_SPEED: f32 = 4.2;
/// Дистанция поимки: безумие мгновенно вскипает до порога.
const MONSTER_CATCH_RADIUS: f32 = 1.1;
/// Пауза между поимками, секунды.
const MONSTER_CATCH_COOLDOWN: f32 = 8.0;
/// Сколько монстр отступает, прежде чем вернуться к охоте.
const MONSTER_RETREAT_SECS: f32 = 2.5;
/// Как часто патруль выбирает новую цель, секунды.
const MONSTER_REPATH_SECS: f32 = 6.0;
/// В Акте 2 монстр замечает игрока ближе этой дистанции (встаёт и смотрит).
const MONSTER_STARE_DIST: f32 = 7.0;

/// Состояние Тени.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonsterState {
    /// Бродит по коридорам (Акт 2).
    Patrol,
    /// Догоняет игрока (Акт 3).
    Chasing,
    /// Бежит от прямого луча фонаря.
    Retreating,
}

/// Тень Отречения: постоянный монстр. Патрулирует (Акт 2), охотится (Акт 3),
/// отступает под прямым лучом фонаря. Коллизии - через общий
/// [`move_with_collision`] (радиус игрока 0.35 монстру тоже подходит).
#[derive(Component)]
pub struct MonsterFade {
    pub state: MonsterState,
    /// Текущая скорость (по состоянию).
    pub speed: f32,
    patrol_target: Vec3,
    state_timer: f32,
    catch_cooldown: f32,
}

/// Спавн Тени: высокая чёрная фигура с красными глазами и багровым светом.
pub(crate) fn spawn_monster(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    spot: Vec3,
    act: GameState,
) {
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
    commands
        .spawn((
            Transform::from_translation(spot),
            MonsterFade {
                state: MonsterState::Patrol,
                speed: MONSTER_PATROL_SPEED,
                patrol_target: spot,
                state_timer: MONSTER_REPATH_SECS,
                catch_cooldown: 0.0,
            },
            StateScoped(act),
        ))
        .with_children(|body| {
            // Торс (~2.6 м ростом вместе с головой).
            body.spawn((
                Mesh3d(meshes.add(Capsule3d::new(0.3, 1.2))),
                MeshMaterial3d(skin.clone()),
                Transform::from_xyz(0.0, 1.3, 0.0),
            ));
            // Вытянутая голова.
            body.spawn((
                Mesh3d(meshes.add(Sphere::new(0.24).mesh().uv(16, 12))),
                MeshMaterial3d(skin.clone()),
                Transform {
                    translation: Vec3::new(0.0, 2.35, 0.0),
                    scale: Vec3::new(1.0, 1.4, 0.95),
                    ..default()
                },
            ));
            // Глаза-огоньки на стороне взгляда (локальный -Z - «лицо»).
            for side in [-1.0f32, 1.0] {
                body.spawn((
                    Mesh3d(meshes.add(Sphere::new(0.05).mesh().uv(12, 8))),
                    MeshMaterial3d(eye_mat.clone()),
                    Transform::from_xyz(0.09 * side, 2.43, -0.21),
                ));
            }
            // Длинные руки.
            for side in [-1.0f32, 1.0] {
                body.spawn((
                    Mesh3d(meshes.add(Capsule3d::new(0.07, 1.0))),
                    MeshMaterial3d(skin.clone()),
                    Transform {
                        translation: Vec3::new(0.46 * side, 1.2, 0.0),
                        rotation: Quat::from_rotation_z(0.15 * -side),
                        ..default()
                    },
                ));
            }
            // Багровый свет-«аура»: Тень видно даже в темноте.
            body.spawn((
                PointLight {
                    color: Color::srgb(1.0, 0.08, 0.06),
                    intensity: 25_000.0,
                    range: 5.0,
                    ..default()
                },
                Transform::from_xyz(0.0, 1.6, 0.0),
            ));
        });
}

/// ИИ Тени: патруль (Акт 2) / охота (Акт 3) / отступление под лучом.
/// Поимка вскипает безумие до порога - дальше штатный триггер скримера.
fn monster_ai(
    time: Res<Time>,
    game_state: Res<State<GameState>>,
    mut commands: Commands,
    colliders: Res<LevelColliders>,
    players: Query<&Transform, With<Player>>,
    lamps: Query<&Flashlight>,
    mut insanity_query: Query<&mut Insanity, With<Player>>,
    mut monsters: Query<(&mut Transform, &mut MonsterFade), Without<Player>>,
) {
    let act = *game_state.get();
    if act != GameState::Act2_TheInsanity && act != GameState::Act3_TheReactor {
        return;
    }
    let Some(player_tf) = players.iter().next() else {
        return;
    };
    let lamp_on = lamps.iter().next().map(|lamp| lamp.is_on).unwrap_or(false);
    let player_pos = player_tf.translation;
    // Луч фонаря: только горизонтальная проекция (взгляд всегда почти горизонтален).
    let forward = player_tf.rotation * Vec3::NEG_Z;
    let flat_forward = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
    let dt = time.delta_secs();
    let t = time.elapsed_secs();
    for (mut transform, mut monster) in &mut monsters {
        monster.state_timer += dt;
        monster.catch_cooldown = (monster.catch_cooldown - dt).max(0.0);
        let to_monster = transform.translation - player_pos;
        let flat = Vec3::new(to_monster.x, 0.0, to_monster.z);
        let flat_dist = flat.length();
        let dir = flat / flat_dist.max(0.001);
        // Прямой луч: фонарь включён, монстр в конусе и в дальности, стена не мешает.
        let lit = lamp_on
            && flat_dist < crate::player::FLASHLIGHT_RANGE
            && flat_forward.dot(dir) > crate::player::FLASHLIGHT_OUTER_ANGLE.cos()
            && has_line_of_sight(player_pos, transform.translation, &colliders.walls);
        if lit && monster.state != MonsterState::Retreating {
            monster.state = MonsterState::Retreating;
            monster.state_timer = 0.0;
            monster.speed = MONSTER_RETREAT_SPEED;
        }
        match monster.state {
            MonsterState::Retreating => {
                // Бежит от игрока.
                let away = -dir;
                move_with_collision(
                    &mut transform.translation,
                    away * monster.speed * dt,
                    &colliders.walls,
                );
                let facing = transform.translation + away;
                face_towards(&mut transform, facing);
                if monster.state_timer >= MONSTER_RETREAT_SECS {
                    monster.state = if act == GameState::Act3_TheReactor {
                        MonsterState::Chasing
                    } else {
                        MonsterState::Patrol
                    };
                    monster.state_timer = 0.0;
                }
            }
            MonsterState::Chasing => {
                if act != GameState::Act3_TheReactor {
                    // Вне охоты погони нет.
                    monster.state = MonsterState::Patrol;
                    monster.state_timer = 0.0;
                } else {
                    monster.speed = MONSTER_CHASE_SPEED;
                    move_with_collision(
                        &mut transform.translation,
                        dir * monster.speed * dt,
                        &colliders.walls,
                    );
                    face_towards(&mut transform, player_pos);
                }
            }
            MonsterState::Patrol => {
                monster.speed = MONSTER_PATROL_SPEED;
                if act == GameState::Act3_TheReactor {
                    // В третьем акте патруля нет - сразу охота.
                    monster.state = MonsterState::Chasing;
                    monster.state_timer = 0.0;
                } else if act == GameState::Act2_TheInsanity && flat_dist < MONSTER_STARE_DIST {
                    // Стоит и смотрит. Жутко.
                    face_towards(&mut transform, player_pos);
                } else if monster.state_timer >= MONSTER_REPATH_SECS
                    || (transform.translation - monster.patrol_target).length() < 0.6
                {
                    let mut rng = rand::thread_rng();
                    let ang = rng.gen_range(0.0..std::f32::consts::TAU);
                    let r = rng.gen_range(4.0..10.0);
                    monster.patrol_target =
                        transform.translation + Vec3::new(ang.cos() * r, 0.0, ang.sin() * r);
                    monster.state_timer = 0.0;
                } else {
                    let to_target = monster.patrol_target - transform.translation;
                    let step = Vec3::new(to_target.x, 0.0, to_target.z);
                    if step.length() > 0.05 {
                        let step_dir = step.normalize();
                        move_with_collision(
                            &mut transform.translation,
                            step_dir * monster.speed * dt,
                            &colliders.walls,
                        );
                        let facing = transform.translation + step_dir;
                        face_towards(&mut transform, facing);
                    }
                }
            }
        }
        // Поимка в любом состоянии: безумие вскипает, Тень отшатывается.
        if flat_dist < MONSTER_CATCH_RADIUS && monster.catch_cooldown <= 0.0 {
            for mut insanity in &mut insanity_query {
                insanity.0 = crate::hallucinations::INSANITY_THRESHOLD;
            }
            monster.catch_cooldown = MONSTER_CATCH_COOLDOWN;
            monster.state = MonsterState::Retreating;
            monster.state_timer = 0.0;
            monster.speed = MONSTER_RETREAT_SPEED;
            crate::interaction::spawn_subtitle(
                &mut commands,
                "MISHA'S VOICE",
                "Why did you leave me there? I was so cold...",
                3.5,
                act,
            );
            info!("The Fade touches you - madness boils over");
        }
        // Парение над полом + лёгкая дрожь.
        transform.translation.y = 0.12 + (t * 2.1 + transform.translation.x).sin() * 0.06;
    }
}

/// Развернуть корень монстра лицом к точке (рысканье, без крена).
fn face_towards(transform: &mut Transform, target: Vec3) {
    let d = target - transform.translation;
    if d.x * d.x + d.z * d.z > 0.0001 {
        let yaw = (-d.x).atan2(-d.z);
        transform.rotation = Quat::from_rotation_y(yaw);
    }
}

/// Прямая видимость: отрезок не пересекает ни одну стену (по XZ, шаг 0.3 м).
fn has_line_of_sight(from: Vec3, to: Vec3, walls: &[WallAabb]) -> bool {
    let delta = Vec3::new(to.x - from.x, 0.0, to.z - from.z);
    let dist = delta.length();
    if dist < 0.001 {
        return true;
    }
    let steps = (dist / 0.3).ceil() as usize;
    for i in 1..=steps {
        let p = from + delta * (i as f32 / steps as f32);
        for w in walls {
            if p.x > w.min_x && p.x < w.max_x && p.z > w.min_z && p.z < w.max_z {
                return false;
            }
        }
    }
    true
}
