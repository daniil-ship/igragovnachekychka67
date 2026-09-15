//! Интерактив актов: батарейки, кассеты, рычаг, реликвии, субтитры.
//!
//! - Батарейки ([`BatteryItem`]) подбираются автоматически рядом с игроком
//!   и восстанавливают заряд фонаря ([`Flashlight`](crate::player::Flashlight)).
//! - Кассеты ([`AudioCassette`]) активируются клавишей E рядом: играет
//!   лор-аудио (файл опционален - проверяется в момент активации, как PNG
//!   скриммеров) и показываются субтитры с текстом записи.
//! - Рычаг генератора ([`GeneratorLever`]): E в Акте 1 ведёт в Акт 2.
//! - Реликвии брата ([`QuestItem`]): автоподбор, 5 штук - хорошая концовка.
//! - Субтитры - общая плашка для кассет, находок, рычага и заставок актов.

use bevy::audio::Volume;
use bevy::prelude::*;
use bevy::state::state_scoped::StateScoped;

use crate::audio::asset_exists;
use crate::player::{Flashlight, Player};
use crate::{GameState, GameProgress};

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
        "TAPE 1/3 - PROF. RADCHENKO, 1984",
        "The psi-seam turns guilt into living hallucinations. On the margins: a drawing of a drowning child. 'I screamed, and you stayed silent.'",
    ),
    (
        "TAPE 2/3 - EVACUATION RECORD, 1984",
        "Screams. Panic. 'It feeds on our guilt! If you run from it - it grows faster. The only way to stop the Fade: hold direct light on it!'",
    ),
    (
        "TAPE 3/3 - UNKNOWN VOICE",
        "It wears the dark like skin. It cannot stand the light, but it has learned to wait. It is very patient. And so close now.",
    ),
];
/// Сколько реликвий брата спрятано в трёх актах (2 + 2 + 1).
pub(crate) const QUEST_COUNT: usize = 5;
/// Названия реликвий для субтитров находки (индекс - [`QuestItem::index`]).
const QUEST_NAMES: [&str; QUEST_COUNT] = [
    "A rusty toy car. Misha never let it go.",
    "A child drawing: two brothers and a black dog.",
    "A wool scarf. It still smells of snow.",
    "A torn photo: father cut out of the frame.",
    "A small mitten. The second one was lost that day.",
];
/// Дистанция автоподбора реликвии (по горизонтали), метры.
const QUEST_PICKUP_RADIUS: f32 = 1.2;
/// Дистанция дёргания рычага клавишей E (по горизонтали), метры.
const LEVER_USE_RADIUS: f32 = 2.2;

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

/// Личная вещь Миши: автоподбор, считается для хорошей концовки.
#[derive(Component)]
pub(crate) struct QuestItem {
    pub(crate) index: usize,
}

/// Рычаг генератора (Акт 1): E рядом дёргает (один раз) - и ведёт в Акт 2.
#[derive(Component)]
pub(crate) struct GeneratorLever {
    pub(crate) pulled: bool,
}

/// Маркер плашки субтитров (одна за раз - новая затирает старую).
#[derive(Component)]
pub(crate) struct SubtitleOverlay;

/// Таймер жизни плашки субтитров.
#[derive(Component)]
struct SubtitleTimer(Timer);

// ---------------------------------------------------------------------------
// Плагин
// ---------------------------------------------------------------------------

/// Регистрирует подбор, E-взаимодействие, парение и субтитры.
/// E-взаимодействие gated'ится разрешением ввода (на секунду скримера
/// управление заблокировано, как и всё остальное).
pub struct InteractionPlugin;

impl Plugin for InteractionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                battery_pickup_system,
                quest_pickup_system,
                bob_pickups,
                bob_quest,
                tick_subtitles,
            )
                .run_if(crate::in_act),
        )
        .add_systems(
            Update,
            (cassette_interaction_system, lever_interaction_system)
                .run_if(crate::game_input_allowed),
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
) {
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
    game_state: Res<State<GameState>>,
    players: Query<&Transform, With<Player>>,
    mut tapes: Query<(&Transform, &mut AudioCassette)>,
    old_subtitles: Query<Entity, With<SubtitleOverlay>>,
) {
    if !keys.just_pressed(KeyCode::KeyE) {
        return;
    }
    let act = *game_state.get();
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
                    StateScoped(act),
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
            let (title, body) = CASSETTE_LORE[tape.lore_index % CASSETTE_LORE.len()];
            spawn_subtitle(&mut commands, title, body, SUBTITLE_SECS, act);
            return;
        }
    }
}

/// Субтитры: тёмная плашка снизу экрана на несколько секунд.
/// Отправители: кассеты (лор), реликвии (находка), рычаг и акты (цели).
pub(crate) fn spawn_subtitle(
    commands: &mut Commands,
    title: &str,
    body: &str,
    secs: f32,
    act: GameState,
) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(70.0),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            SubtitleTimer(Timer::from_seconds(secs, TimerMode::Once)),
            SubtitleOverlay,
            StateScoped(act),
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
                StateScoped(act),
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
                    StateScoped(act),
                ));
                panel.spawn((
                    Text::new(body),
                    TextFont {
                        font_size: 17.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.85, 0.83, 0.8)),
                    SubtitleOverlay,
                    StateScoped(act),
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

// ---------------------------------------------------------------------------
// Реликвии брата и рычаг генератора
// ---------------------------------------------------------------------------

/// Подбор реликвий брата: счётчик для концовки + короткий субтитр находки.
fn quest_pickup_system(
    mut commands: Commands,
    game_state: Res<State<GameState>>,
    players: Query<&Transform, With<Player>>,
    items: Query<(Entity, &Transform, &QuestItem)>,
    old_subtitles: Query<Entity, With<SubtitleOverlay>>,
    mut progress: ResMut<GameProgress>,
) {
    let act = *game_state.get();
    for player in &players {
        for (entity, transform, item) in &items {
            let dx = player.translation.x - transform.translation.x;
            let dz = player.translation.z - transform.translation.z;
            if dx * dx + dz * dz < QUEST_PICKUP_RADIUS * QUEST_PICKUP_RADIUS {
                commands.entity(entity).despawn();
                progress.quest_items += 1;
                let name = QUEST_NAMES[item.index % QUEST_COUNT];
                // Новая находка - старые субтитры убираем (одна плашка за раз).
                for entity in &old_subtitles {
                    commands.entity(entity).despawn();
                }
                spawn_subtitle(
                    &mut commands,
                    &format!("KEEPSAKE {}/{} FOUND", progress.quest_items, QUEST_COUNT),
                    name,
                    3.0,
                    act,
                );
                info!(
                    "Keepsake found: {name} ({}/{QUEST_COUNT})",
                    progress.quest_items
                );
            }
        }
    }
}

/// Рычаг генератора: E рядом дёргает (один раз - краснеет и зеленеет),
/// через пару секунд - переход в Акт 2.
fn lever_interaction_system(
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    game_state: Res<State<GameState>>,
    players: Query<&Transform, With<Player>>,
    mut levers: Query<(&Transform, &mut GeneratorLever, &MeshMaterial3d<StandardMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !keys.just_pressed(KeyCode::KeyE) {
        return;
    }
    let act = *game_state.get();
    for player in &players {
        for (transform, mut lever, mat) in &mut levers {
            if lever.pulled {
                continue;
            }
            let dx = player.translation.x - transform.translation.x;
            let dz = player.translation.z - transform.translation.z;
            if dx * dx + dz * dz > LEVER_USE_RADIUS * LEVER_USE_RADIUS {
                continue;
            }
            lever.pulled = true;
            if let Some(material) = materials.get_mut(&mat.0) {
                material.base_color = Color::srgb(0.15, 0.7, 0.2);
            }
            commands.insert_resource(crate::PendingTransition {
                timer: Timer::from_seconds(2.5, TimerMode::Once),
                next: crate::GameState::Act2_TheInsanity,
            });
            spawn_subtitle(
                &mut commands,
                "THE GENERATOR ROARS",
                "The lights stutter across the sector. Something shifts in the dark.",
                4.0,
                act,
            );
            info!("Generator lever pulled - Act 2 incoming");
            return;
        }
    }
}

/// Парение и вращение реликвий (чисто визуальное).
fn bob_quest(time: Res<Time>, mut query: Query<&mut Transform, With<QuestItem>>) {
    let t = time.elapsed_secs();
    for mut transform in &mut query {
        transform.translation.y = 0.4 + (t * 2.0 + transform.translation.z).sin() * 0.08;
        transform.rotation = Quat::from_rotation_y(t * 1.3);
    }
}
