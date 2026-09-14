//! Безумие и галлюцинации (скримеры PNG + MP3).
//!
//! Механика:
//! 1. Пока поднят флаг [`ForcedRun`](crate::player::ForcedRun) (бег на пустой
//!    стамине), растёт скрытый параметр [`Insanity`].
//! 2. Триггер скримера срабатывает, когда безумие достигает
//!    [`INSANITY_THRESHOLD`] **или** истекает случайный таймер
//!    принудительного бега (5–10 секунд).
//! 3. Случайно выбирается пара файлов с одинаковым индексом
//!    (`skrimerN.png` + `screamN.mp3`), картинка на **ровно 1 секунду**
//!    рендерится поверх всего (`Timer::from_seconds(1.0, TimerMode::Once)`),
//!    звук орёт на максимальной громкости, управление заблокировано.
//! 4. Через секунду сущность картинки удаляется (`despawn`), безумие
//!    сбрасывается, игра продолжается.
//!
//! Файлы скриммеров опциональны: наличие проверяется при **каждом**
//! срабатывании, поэтому файлы, положенные в `assets/` при запущенной игре,
//! подхватятся автоматически. Если PNG нет — мигает красно-чёрная заглушка,
//! если MP3 нет — скример проходит без звука.

use bevy::audio::Volume;
use bevy::prelude::*;
use bevy::state::state_scoped::StateScoped;
use rand::Rng;

use crate::audio::asset_exists;
use crate::player::{ForcedRun, Player, Stamina};
use crate::AppState;

// ---------------------------------------------------------------------------
// Константы
// ---------------------------------------------------------------------------

/// Картинки скриммеров (пути внутри `assets/`). Индекс связан со звуком.
pub const SCREAMER_IMAGES: [&str; 3] = [
    "screamers/skrimer1.png",
    "screamers/skrimer2.png",
    "screamers/skrimer3.png",
];
/// Звуки скриммеров (пути внутри `assets/`). Индекс связан с картинкой.
pub const SCREAMER_SOUNDS: [&str; 3] = [
    "audio/scream1.mp3",
    "audio/scream2.mp3",
    "audio/scream3.mp3",
];

/// Критический порог безумия: при достижении срабатывает скример.
pub const INSANITY_THRESHOLD: f32 = 100.0;
/// Скорость накопления безумия в принудительном беге (ед/с).
const INSANITY_GAIN_PER_SEC: f32 = 14.0;
/// Скорость «остывания» безумия, когда герой не насилует себя (ед/с).
const INSANITY_DECAY_PER_SEC: f32 = 8.0;
/// Сколько секунд скример висит на экране. РОВНО 1.0 по ТЗ.
const SCREAMER_DURATION_SECS: f32 = 1.0;
/// Пауза после скримера, во время которой новый триггер невозможен.
const TRIGGER_COOLDOWN_SECS: f32 = 3.0;
/// Границы случайного таймера принудительного бега, секунды.
const RANDOM_TRIGGER_MIN_SECS: f32 = 5.0;
const RANDOM_TRIGGER_MAX_SECS: f32 = 10.0;
/// Сколько стамины возвращается герою после пережитого скримера.
const STAMINA_AFTER_SCREAMER: f32 = 30.0;

// ---------------------------------------------------------------------------
// Компоненты и ресурсы
// ---------------------------------------------------------------------------

/// Скрытый параметр безумия 0..100. Числа игрок не видит — только эффекты:
/// виньетку, мерцание фонарика и, в конце концов, скример.
#[derive(Component)]
pub struct Insanity(pub f32);

/// Маркер оверлея скримера (картинка или заглушка на весь экран).
/// Виден внутри крейта: нужен для чистки при рестарте забега.
#[derive(Component)]
pub(crate) struct ScreamerOverlay;

/// Маркер заглушки: PNG-файл отсутствует, мигаем цветом.
#[derive(Component)]
struct FallbackFlash;

/// Состояние системы скриммеров.
#[derive(Resource)]
pub struct ScreamerState {
    /// `true`, пока скример висит на экране (управление заблокировано).
    pub active: bool,
    /// Односекундный таймер жизни скримера.
    timer: Timer,
    /// Остаток кулдауна после прошлого скримера.
    cooldown: f32,
    /// Случайный таймер: срабатывает, если принудительный бег длится
    /// достаточно долго (пересоздаётся при каждом новом забеге).
    random_trigger: Option<Timer>,
}

impl Default for ScreamerState {
    fn default() -> Self {
        Self {
            active: false,
            timer: Timer::from_seconds(SCREAMER_DURATION_SECS, TimerMode::Once),
            cooldown: 0.0,
            random_trigger: None,
        }
    }
}

impl ScreamerState {
    /// Полный сброс (используется при рестарте забега клавишей R).
    pub fn full_reset(&mut self) {
        self.active = false;
        self.cooldown = 0.0;
        self.random_trigger = None;
    }
}

// ---------------------------------------------------------------------------
// Плагин
// ---------------------------------------------------------------------------

/// Регистрирует накопление безумия и тиканье скримера.
pub struct HallucinationsPlugin;

impl Plugin for HallucinationsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ScreamerState::default()).add_systems(
            Update,
            (update_madness, tick_screamer).run_if(in_state(AppState::InGame)),
        );
    }
}

// ---------------------------------------------------------------------------
// Системы
// ---------------------------------------------------------------------------

/// Накопление/затухание безумия и триггер скримера.
///
/// Триггер срабатывает при выполнении ЛЮБОГО из условий:
/// - безумие достигло [`INSANITY_THRESHOLD`];
/// - истёк случайный таймер принудительного бега (5–10 с).
fn update_madness(
    time: Res<Time>,
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut state: ResMut<ScreamerState>,
    mut player_query: Query<(&ForcedRun, &mut Insanity)>,
) {
    let dt = time.delta_secs();
    state.cooldown = (state.cooldown - dt).max(0.0);
    if state.active {
        return;
    }

    for (forced, mut insanity) in &mut player_query {
        if forced.0 && state.cooldown <= 0.0 {
            // 1) Безумие копится, пока герой бежит из последних сил.
            insanity.0 = (insanity.0 + INSANITY_GAIN_PER_SEC * dt).min(INSANITY_THRESHOLD);

            // 2) Случайный таймер принудительного бега.
            let mut rng = rand::thread_rng();
            let time_up = if let Some(timer) = state.random_trigger.as_mut() {
                timer.tick(time.delta());
                timer.finished()
            } else {
                state.random_trigger = Some(Timer::from_seconds(
                    rng.gen_range(RANDOM_TRIGGER_MIN_SECS..RANDOM_TRIGGER_MAX_SECS),
                    TimerMode::Once,
                ));
                false
            };

            // 3) Триггер: порог безумия ИЛИ случайный таймер.
            if insanity.0 >= INSANITY_THRESHOLD || time_up {
                fire_screamer(&mut commands, &assets, &mut state);
                state.random_trigger = None;
            }
        } else {
            // Отдых: безумие медленно отпускает, случайный таймер сбрасывается.
            insanity.0 = (insanity.0 - INSANITY_DECAY_PER_SEC * dt).max(0.0);
            state.random_trigger = None;
        }
    }
}

/// Срабатывание скримера: выбор случайной пары файлов, картинка на весь
/// экран, звук на максимальной громкости, запуск односекундного таймера.
fn fire_screamer(
    commands: &mut Commands,
    assets: &AssetServer,
    state: &mut ScreamerState,
) {
    // Случайная пара с одинаковым индексом: skrimerN.png + screamN.mp3.
    let mut rng = rand::thread_rng();
    let index = rng.gen_range(0..SCREAMER_IMAGES.len());
    let image_path = SCREAMER_IMAGES[index];
    let sound_path = SCREAMER_SOUNDS[index];

    // --- Картинка поверх всего интерфейса (UI-нода на весь экран) ---
    // Спавнится позже всего UI, поэтому рендерится самой верхней.
    let fullscreen = Node {
        width: Val::Percent(100.0),
        height: Val::Percent(100.0),
        ..default()
    };
    if asset_exists(image_path) {
        let handle: Handle<Image> = assets.load(image_path);
        commands.spawn((
            fullscreen,
            ImageNode::new(handle),
            ScreamerOverlay,
            StateScoped(AppState::InGame),
        ));
    } else {
        // PNG нет — игра продолжается, мигает красно-чёрная заглушка.
        // Как только файл появится в assets/, будет использоваться он.
        warn!("{image_path} not found — using fallback flash (drop the file into assets/ to enable the real screamer)");
        commands.spawn((
            fullscreen,
            BackgroundColor(Color::srgb(0.6, 0.0, 0.0)),
            FallbackFlash,
            ScreamerOverlay,
            StateScoped(AppState::InGame),
        ));
    }

    // --- Звук на максимальной громкости (линейная 1.0 = оригинал) ---
    if asset_exists(sound_path) {
        let handle: Handle<AudioSource> = assets.load(sound_path);
        commands.spawn((
            AudioPlayer::new(handle),
            PlaybackSettings::DESPAWN.with_volume(Volume::Linear(1.0)),
            StateScoped(AppState::InGame),
        ));
    } else {
        warn!("{sound_path} not found — screamer without sound (drop the file into assets/ to enable it)");
    }

    // --- Запуск таймера ровно на 1 секунду ---
    state.active = true;
    state.timer = Timer::from_seconds(SCREAMER_DURATION_SECS, TimerMode::Once);
    info!("SCREAMER! (pair #{})", index + 1);
}

/// Тиканье активного скримера: мигание заглушки, тряска камеры и —
/// ровно через секунду — удаление картинки и возврат в игру.
fn tick_screamer(
    time: Res<Time>,
    mut commands: Commands,
    mut state: ResMut<ScreamerState>,
    overlay_query: Query<Entity, With<ScreamerOverlay>>,
    mut flash_query: Query<&mut BackgroundColor, With<FallbackFlash>>,
    mut camera_query: Query<&mut Transform, With<Player>>,
    mut player_query: Query<(&mut Insanity, &mut Stamina)>,
) {
    if !state.active {
        return;
    }

    // Тикаем односекундный таймер.
    state.timer.tick(time.delta());

    // Заглушка (если PNG не было) мигает ~12 раз в секунду.
    let bright_phase = (state.timer.elapsed_secs() * 12.0) as i32 % 2 == 0;
    for mut color in &mut flash_query {
        color.0 = if bright_phase {
            Color::srgb(0.7, 0.0, 0.0)
        } else {
            Color::srgb(0.05, 0.0, 0.0)
        };
    }

    // Камера дрожит, пока скример на экране.
    let mut rng = rand::thread_rng();
    for mut transform in &mut camera_query {
        transform.translation.x += rng.gen_range(-0.03..0.03);
        transform.translation.z += rng.gen_range(-0.03..0.03);
    }

    // Ровно через 1 секунду: убрать картинку, сбросить безумие, играть дальше.
    if state.timer.just_finished() {
        for entity in &overlay_query {
            commands.entity(entity).despawn();
        }
        for (mut insanity, mut stamina) in &mut player_query {
            insanity.0 = 0.0;
            stamina.current = STAMINA_AFTER_SCREAMER;
        }
        state.active = false;
        state.cooldown = TRIGGER_COOLDOWN_SECS;
        info!("Screamer over — back to the dark.");
    }
}
