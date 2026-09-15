//! Эффекты страха и физика света ([`HorrorEffectsPlugin`]).
//!
//! - Инерция фонаря: свет запаздывает за поворотами камеры (lerp от угловой
//!   скорости) + синусоидальная дрожь рук, растущая с безумием.
//! - Моргание севшей батареи: заряд < 20 - хаотичные вспышки поверх базового
//!   мерцания (система идёт строго после [`flashlight_flicker`](crate::player::flashlight_flicker)).
//! - Туннельное зрение: в принудительном беге или при безумии > 60 FOV камеры
//!   плавно сужается, на отдыхе возвращается в норму.
//! - Шаги: тихий топот при движении (файлы опциональны).
//! - Звуковые аномалии: при безумии > 50 позади игрока слышны шаги и шёпот
//!   (пространственный 3D-звук, файлы опциональны).

use bevy::audio::Volume;
use bevy::prelude::*;
use bevy::state::state_scoped::StateScoped;
use rand::Rng;

use crate::audio::asset_exists;
use crate::hallucinations::Insanity;
use crate::player::{Flashlight, ForcedRun, Player};

/// Насколько сильно фонарь отстаёт от поворотов (доля угловой скорости).
const SWAY_FACTOR: f32 = 0.012;
/// Максимальное отклонение фонаря от взгляда, радианы.
const SWAY_MAX: f32 = 0.06;
/// Скорость догоняния фонарём камеры (lerp/ед. времени).
const SWAY_LERP_RATE: f32 = 8.0;
/// Максимальная дрожь рук при безумии 100, радианы.
const TREMOR_MAX: f32 = 0.02;
/// Ниже этого заряда батарея моргает хаотично.
const BATTERY_FLICKER_BELOW: f32 = 20.0;
/// Безумие, выше которого сужается FOV.
const TUNNEL_INSANITY: f32 = 60.0;
/// FOV в панике, радианы (норма берётся с камеры при старте).
const TUNNEL_FOV_MIN: f32 = 0.55;
/// Скорость сужения/возврата FOV.
const TUNNEL_LERP_RATE: f32 = 3.0;
/// Безумие, выше которого начинаются звуковые аномалии.
const ANOMALY_INSANITY: f32 = 50.0;
/// Шаги героя (пути внутри `assets/`, файлы опциональны).
const FOOTSTEP_SOUNDS: [&str; 2] = ["audio/step1.mp3", "audio/step2.mp3"];
/// Аномалии позади игрока (пути внутри `assets/`, файлы опциональны).
const ANOMALY_SOUNDS: [&str; 3] = [
    "audio/whisper1.mp3",
    "audio/whisper2.mp3",
    "audio/step_behind.mp3",
];

// ---------------------------------------------------------------------------
// Плагин
// ---------------------------------------------------------------------------

/// Регистрирует эффекты страха. Моргание батареи идёт строго после базового
/// мерцания фонаря (оно выставляет интенсивность абсолютно каждый кадр).
pub struct HorrorEffectsPlugin;

impl Plugin for HorrorEffectsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(FlashlightSway::default())
            .insert_resource(FootstepState::default())
            .insert_resource(AnomalyState::default())
            .add_systems(Update, sway_flashlight.run_if(crate::in_act))
            .add_systems(Update, tunnel_vision.run_if(crate::in_act))
            .add_systems(
                Update,
                battery_flicker
                    .run_if(crate::game_input_allowed)
                    .after(crate::player::flashlight_flicker),
            )
            .add_systems(Update, footsteps.run_if(crate::game_input_allowed))
            .add_systems(Update, sound_anomalies.run_if(crate::in_act));
    }
}

// ---------------------------------------------------------------------------
// Инерция фонаря и дрожь рук
// ---------------------------------------------------------------------------

/// Состояние инерции фонаря: прошлый угол камеры и текущее отставание.
#[derive(Resource, Default)]
struct FlashlightSway {
    prev_yaw: f32,
    prev_pitch: f32,
    off_yaw: f32,
    off_pitch: f32,
    initialized: bool,
}

/// Инерция фонаря (запаздывание за камерой) + дрожь рук от безумия.
/// Пишет только локальный поворот ребёнка-фонаря - базовый свет не трогаем.
fn sway_flashlight(
    time: Res<Time>,
    mut sway: ResMut<FlashlightSway>,
    players: Query<(&Player, &Insanity)>,
    mut lamps: Query<&mut Transform, With<Flashlight>>,
) {
    let Some((player, insanity)) = players.iter().next() else {
        return;
    };
    let dt = time.delta_secs().max(0.0001);
    if !sway.initialized {
        sway.prev_yaw = player.yaw;
        sway.prev_pitch = player.pitch;
        sway.initialized = true;
    }
    // Угловая скорость камеры.
    let yaw_vel = (player.yaw - sway.prev_yaw) / dt;
    let pitch_vel = (player.pitch - sway.prev_pitch) / dt;
    sway.prev_yaw = player.yaw;
    sway.prev_pitch = player.pitch;
    // Цель: отклонение против движения (инерция); в покое цель - ноль.
    let target_yaw = (-yaw_vel * SWAY_FACTOR).clamp(-SWAY_MAX, SWAY_MAX);
    let target_pitch = (-pitch_vel * SWAY_FACTOR).clamp(-SWAY_MAX, SWAY_MAX);
    let k = (dt * SWAY_LERP_RATE).min(1.0);
    sway.off_yaw += (target_yaw - sway.off_yaw) * k;
    sway.off_pitch += (target_pitch - sway.off_pitch) * k;
    // Дрожь рук: синусоидальный микро-шум, растёт с квадратом безумия.
    let madness = (insanity.0 / 100.0).clamp(0.0, 1.0);
    let t = time.elapsed_secs();
    let tremor = madness * madness * TREMOR_MAX;
    let ty = ((t * 39.0).sin() * 0.6 + (t * 61.0 + 1.3).sin() * 0.4) * tremor;
    let tp = ((t * 47.0 + 2.1).sin() * 0.6 + (t * 71.0 + 0.7).sin() * 0.4) * tremor;
    for mut transform in &mut lamps {
        transform.rotation = Quat::from_euler(
            EulerRot::YXZ,
            sway.off_yaw + ty,
            sway.off_pitch + tp,
            sway.off_yaw * 0.3 + ty * 0.5,
        );
    }
}

// ---------------------------------------------------------------------------
// Моргание севшей батареи
// ---------------------------------------------------------------------------

/// Севшая батарея: хаотичное моргание поверх базового мерцания.
/// Умножает уже выставленную интенсивность - поэтому только после
/// `flashlight_flicker` и только пока он работает (тот же run_if).
fn battery_flicker(mut lamps: Query<(&Flashlight, &mut SpotLight)>) {
    let mut rng = rand::thread_rng();
    for (lamp, mut light) in &mut lamps {
        if lamp.is_on && lamp.battery < BATTERY_FLICKER_BELOW {
            light.intensity *= rng.gen_range(0.15..1.0);
        }
    }
}

// ---------------------------------------------------------------------------
// Туннельное зрение
// ---------------------------------------------------------------------------

/// Туннельное зрение: в принудительном беге или при безумии > 60 FOV плавно
/// сужается, на отдыхе возвращается к норме с камеры.
fn tunnel_vision(
    time: Res<Time>,
    mut base_fov: Local<Option<f32>>,
    mut query: Query<(&ForcedRun, &Insanity, &mut Projection), With<Player>>,
) {
    for (forced, insanity, mut proj) in &mut query {
        if let Projection::Perspective(persp) = &mut *proj {
            let base = *base_fov.get_or_insert(persp.fov);
            let panic = forced.0 || insanity.0 > TUNNEL_INSANITY;
            let target = if panic { TUNNEL_FOV_MIN } else { base };
            let k = (time.delta_secs() * TUNNEL_LERP_RATE).min(1.0);
            persp.fov += (target - persp.fov) * k;
        }
    }
}

// ---------------------------------------------------------------------------
// Шаги и звуковые аномалии
// ---------------------------------------------------------------------------

/// Состояние шагов: обратный отсчёт до следующего топота.
#[derive(Resource, Default)]
struct FootstepState {
    timer: f32,
}

/// Шаги героя: тихий топот при движении, на бегу чаще и громче.
/// Молчит без файлов (как кассеты и скримеры).
fn footsteps(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    assets: Res<AssetServer>,
    game_state: Res<State<crate::GameState>>,
    mut steps: ResMut<FootstepState>,
) {
    let moving = keys.pressed(KeyCode::KeyW)
        || keys.pressed(KeyCode::KeyA)
        || keys.pressed(KeyCode::KeyS)
        || keys.pressed(KeyCode::KeyD)
        || keys.pressed(KeyCode::ArrowUp)
        || keys.pressed(KeyCode::ArrowLeft)
        || keys.pressed(KeyCode::ArrowDown)
        || keys.pressed(KeyCode::ArrowRight);
    if !moving {
        steps.timer = 0.0;
        return;
    }
    steps.timer -= time.delta_secs();
    if steps.timer > 0.0 {
        return;
    }
    let running = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    steps.timer = if running { 0.32 } else { 0.48 };
    let mut rng = rand::thread_rng();
    let path = FOOTSTEP_SOUNDS[rng.gen_range(0..FOOTSTEP_SOUNDS.len())];
    if !asset_exists(path) {
        return;
    }
    let handle: Handle<AudioSource> = assets.load(path);
    commands.spawn((
        AudioPlayer::new(handle),
        PlaybackSettings::DESPAWN.with_volume(Volume::Linear(if running { 0.35 } else { 0.25 })),
        StateScoped(*game_state.get()),
    ));
}

/// Состояние аномалий: обратный отсчёт до следующего шёпота.
#[derive(Resource)]
struct AnomalyState {
    countdown: f32,
}

impl Default for AnomalyState {
    fn default() -> Self {
        Self { countdown: 6.0 }
    }
}

/// Звуковые аномалии: при безумии > 50 позади игрока случайно слышны шаги
/// и шёпот (пространственный 3D-звук через `SpatialListener` на камере).
/// Молчит без файлов (как кассеты и скримеры).
fn sound_anomalies(
    time: Res<Time>,
    mut commands: Commands,
    assets: Res<AssetServer>,
    game_state: Res<State<crate::GameState>>,
    players: Query<(&Transform, &Insanity), With<Player>>,
    mut anomaly: ResMut<AnomalyState>,
) {
    let Some((transform, insanity)) = players.iter().next() else {
        return;
    };
    if insanity.0 <= ANOMALY_INSANITY {
        anomaly.countdown = 4.0;
        return;
    }
    anomaly.countdown -= time.delta_secs();
    if anomaly.countdown > 0.0 {
        return;
    }
    let mut rng = rand::thread_rng();
    anomaly.countdown = rng.gen_range(6.0..14.0);
    // Позади игрока: против взгляда + случайный боковой сдвиг.
    let fwd = transform.rotation * Vec3::NEG_Z;
    let back = Vec3::new(-fwd.x, 0.0, -fwd.z).normalize_or_zero();
    let right = Vec3::new(-back.z, 0.0, back.x);
    let dist = rng.gen_range(2.5..5.0);
    let side = rng.gen_range(-2.0..2.0);
    let pos = transform.translation + back * dist + right * side;
    let pos = Vec3::new(pos.x, 1.3, pos.z);
    let path = ANOMALY_SOUNDS[rng.gen_range(0..ANOMALY_SOUNDS.len())];
    if !asset_exists(path) {
        return;
    }
    let handle: Handle<AudioSource> = assets.load(path);
    let mut playback = PlaybackSettings::DESPAWN.with_volume(Volume::Linear(0.8));
    playback.spatial = true;
    commands.spawn((
        AudioPlayer::new(handle),
        playback,
        Transform::from_translation(pos),
        StateScoped(*game_state.get()),
    ));
}
