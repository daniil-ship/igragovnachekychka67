//! Подбор батареек и сюжетные кассеты (механики Акта 2).
//!
//! - Батарейки ([`BatteryItem`]) подбираются автоматически рядом с игроком
//!   и восстанавливают заряд фонаря ([`Flashlight`](crate::player::Flashlight)).
//! - Кассеты ([`AudioCassette`]) активируются клавишей E рядом: играет
//!   лор-аудио (файл опционален - проверяется в момент активации, как PNG
//!   скриммеров) и показываются субтитры с текстом записи.
//! Точки спавна хранятся в [`GameProgress`](crate::GameProgress) для рестарта.

use bevy::audio::Volume;
use bevy::prelude::*;
use bevy::state::state_scoped::StateScoped;

use crate::audio::asset_exists;
use crate::player::{Flashlight, Player};
use crate::{AppState, GameProgress};

// ---------------------------------------------------------------------------
// Константы баланса
// ---------------------------------------------------------------------------

/// Сколько батареек разбросано по уровню.
pub(crate) const BATTERY_COUNT: usize = 6;
/// Сколько заряда восстанавливает одна батарейка.
pub(crate) const BATTERY_RECHARGE: f32 = 40.0;
/// Дистанция подбора батарейки (по горизонтали), метры.
const BATTERY_PICKUP_RADIUS: f32 = 1.2;
/// Сколько сюжетных кассет на уровень.
pub(crate) const CASSETTE_COUNT: usize = 3;
/// Лор-аудио кассет (пути внутри `assets/`). Индекс связан с субтитрами.
/// Файлы опциональны: без них кассета играет немо, только субтитры.
pub(crate) const CASSETTE_TRACKS: [&str; CASSETTE_COUNT] = [
    "audio/cassette1.mp3",
    "audio/cassette2.mp3",
    "audio/cassette3.mp3",
];
/// Дистанция активации кассеты клавишей E (по горизонтали), метры.
const CASSETTE_USE_RADIUS: f32 = 2.2;
/// Сколько секунд висят субтитры кассеты.
const SUBTITLE_SECS: f32 = 7.0;
/// Субтитры кассет: (заголовок, текст). Только ASCII - шрифт Bevy
/// не знает тире и кавычек-ёлочек.
const CASSETTE_LORE: [(&str, &str); CASSETTE_COUNT] = [
    (
        "TAPE 1/3 - DR. VELSKAYA, DAY 12",
        "The walls breathe when the lights die. I counted four breaths between the flickers. Whatever you do - do not let your torch go out.",
    ),
    (
        "TAPE 2/3 - ORDERLY GRIMM, DAY 27",
        "Batteries drain faster near the east cells. As if something down there is thirsty. We stopped going there alone.",
    ),
    (
        "TAPE 3/3 - UNKNOWN VOICE, DAY 40",
        "It wears the dark like skin. It cannot stand the light, but it has learned to wait. It is very, very patient. And so close now.",
    ),
];

// ---------------------------------------------------------------------------
// Компоненты
// ---------------------------------------------------------------------------

/// Батарейка: подбор восстанавливает заряд фонаря (не выше максимума).
#[derive(Component)]
pub(crate) struct BatteryItem {
    pub(crate) recharge_amount: f32,
}

/// Сюжетная кассета: E рядом включает запись (один раз) + субтитры с лором.
#[derive(Component)]
pub(crate) struct AudioCassette {
    pub(crate) audio_path: String,
    pub(crate) was_played: bool,
    pub(crate) lore_index: usize,
}

/// Маркер плашки субтитров (для удаления при рестарте и затирании).
#[derive(Component)]
pub(crate) struct SubtitleOverlay;

/// Таймер жизни плашки субтитров.
#[derive(Component)]
struct SubtitleTimer(Timer);

// ---------------------------------------------------------------------------
// Плагин
// ---------------------------------------------------------------------------

/// Регистрирует подбор батареек, кассеты, парение и субтитры.
/// E-взаимодействие gated'ится разрешением ввода (на секунду скримера
/// управление заблокировано, как и всё остальное).
pub struct InteractionPlugin;

impl Plugin for InteractionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (battery_pickup_system, bob_pickups, tick_subtitles)
                .run_if(in_state(AppState::InGame)),
        )
        .add_systems(
            Update,
            cassette_interaction_system.run_if(crate::game_input_allowed),
        );
    }
}

// ---------------------------------------------------------------------------
// Системы
// ---------------------------------------------------------------------------

/// Подбор батареек при приближении (как фрагменты эха, но для фонаря).
fn battery_pickup_system(
    mut commands: Commands,
    players: Query<&Transform, With<Player>>,
    items: Query<(Entity, &Transform, &BatteryItem)>,
    mut lamps: Query<&mut Flashlight>,
    progress: Res<GameProgress>,
) {
    if progress.won || progress.dead {
        return;
    }
    for player in &players {
        for (entity, transform, item) in &items {
            let dx = player.translation.x - transform.translation.x;
            let dz = player.translation.z - transform.translation.z;
            if dx * dx + dz * dz < BATTERY_PICKUP_RADIUS * BATTERY_PICKUP_RADIUS {
                commands.entity(entity).despawn();
                for mut lamp in &mut lamps {
                    lamp.battery =
                        (lamp.battery + item.recharge_amount).min(Flashlight::MAX_BATTERY);
                }
                info!("Battery picked up (+{} charge)", item.recharge_amount);
            }
        }
    }
}

/// Взаимодействие с кассетами: E рядом с непроигранной включает запись
/// (один раз) и показывает субтитры. Один нажим - одна кассета.
fn cassette_interaction_system(
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    assets: Res<AssetServer>,
    players: Query<&Transform, With<Player>>,
    mut tapes: Query<(&Transform, &mut AudioCassette)>,
    old_subtitles: Query<Entity, With<SubtitleOverlay>>,
) {
    if !keys.just_pressed(KeyCode::KeyE) {
        return;
    }
    for player in &players {
        for (transform, mut tape) in &mut tapes {
            if tape.was_played {
                continue;
            }
            let dx = player.translation.x - transform.translation.x;
            let dz = player.translation.z - transform.translation.z;
            if dx * dx + dz * dz > CASSETTE_USE_RADIUS * CASSETTE_USE_RADIUS {
                continue;
            }
            tape.was_played = true;
            if asset_exists(&tape.audio_path) {
                let handle: Handle<AudioSource> = assets.load(tape.audio_path.clone());
                commands.spawn((
                    AudioPlayer::new(handle),
                    PlaybackSettings::DESPAWN.with_volume(Volume::Linear(1.0)),
                    StateScoped(AppState::InGame),
                ));
                info!("Playing {}", tape.audio_path);
            } else {
                warn!(
                    "{} not found - cassette plays silent, subtitles only (drop the file into assets/ to enable it)",
                    tape.audio_path
                );
            }
            // Новая запись - старые субтитры убираем (одна плашка за раз).
            for entity in &old_subtitles {
                commands.entity(entity).despawn();
            }
            spawn_subtitle(&mut commands, tape.lore_index);
            return;
        }
    }
}

/// Субтитры кассеты: тёмная плашка с лором снизу экрана на несколько секунд.
fn spawn_subtitle(commands: &mut Commands, lore_index: usize) {
    let (title, body) = CASSETTE_LORE[lore_index % CASSETTE_LORE.len()];
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(70.0),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            SubtitleTimer(Timer::from_seconds(SUBTITLE_SECS, TimerMode::Once)),
            SubtitleOverlay,
            StateScoped(AppState::InGame),
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(6.0),
                    padding: UiRect {
                        left: Val::Px(24.0),
                        right: Val::Px(24.0),
                        top: Val::Px(12.0),
                        bottom: Val::Px(12.0),
                    },
                    max_width: Val::Px(640.0),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.75)),
                SubtitleOverlay,
                StateScoped(AppState::InGame),
            ))
            .with_children(|panel| {
                panel.spawn((
                    Text::new(title),
                    TextFont {
                        font_size: 20.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.95, 0.75, 0.4)),
                    SubtitleOverlay,
                    StateScoped(AppState::InGame),
                ));
                panel.spawn((
                    Text::new(body),
                    TextFont {
                        font_size: 17.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.85, 0.83, 0.8)),
                    SubtitleOverlay,
                    StateScoped(AppState::InGame),
                ));
            });
        });
}

/// Тиканье субтитров: по окончании таймера плашка удаляется целиком.
/// (Субтитр всегда один - новый затирает старый, см. cassette_interaction_system.)
fn tick_subtitles(
    time: Res<Time>,
    mut commands: Commands,
    mut timers: Query<&mut SubtitleTimer>,
    all: Query<Entity, With<SubtitleOverlay>>,
) {
    for mut timer in &mut timers {
        timer.0.tick(time.delta());
    }
    let done = timers.iter().any(|timer| timer.0.just_finished());
    if done {
        for entity in &all {
            commands.entity(entity).despawn();
        }
    }
}

/// Парение и вращение батареек и кассет (чисто визуальное).
fn bob_pickups(
    time: Res<Time>,
    mut batteries: Query<&mut Transform, (With<BatteryItem>, Without<AudioCassette>)>,
    mut tapes: Query<&mut Transform, (With<AudioCassette>, Without<BatteryItem>)>,
) {
    let t = time.elapsed_secs();
    for mut transform in &mut batteries {
        transform.translation.y = 0.35 + (t * 2.2 + transform.translation.x).sin() * 0.08;
        transform.rotation = Quat::from_rotation_y(t * 1.6);
    }
    for mut transform in &mut tapes {
        transform.translation.y = 0.30 + (t * 1.8 + transform.translation.z).sin() * 0.07;
        transform.rotation = Quat::from_rotation_y(t * 1.1);
    }
}
