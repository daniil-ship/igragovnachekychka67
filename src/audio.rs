//! Звукорежиссёр: музыка меню и циклический амбиент с плавными переходами.
//!
//! - В [`AppState::MainMenu`](crate::AppState::MainMenu) играет `menu.mp3`.
//! - В [`AppState::InGame`](crate::AppState::InGame) циклично сменяют друг
//!   друга `ambient1.mp3` и `ambient2.mp3` (каждый с плавным нарастанием,
//!   переключение - бесшовное, по факту окончания трека).
//!
//! Все аудиофайлы **опциональны**: если их нет в `assets/`, игра просто
//! работает без звука и автоматически подхватывает файлы, когда они там
//! появятся (повторная проверка каждые [`RETRY_SECS`] секунд).

use std::path::Path;

use bevy::audio::Volume;
use bevy::prelude::*;

use crate::AppState;

// ---------------------------------------------------------------------------
// Константы
// ---------------------------------------------------------------------------

/// Музыка главного меню (путь внутри `assets/`).
pub const MENU_MUSIC: &str = "audio/menu.mp3";
/// Фоновые амбиенты игры, играют по очереди (пути внутри `assets/`).
pub const AMBIENT_TRACKS: [&str; 2] = ["audio/ambient1.mp3", "audio/ambient2.mp3"];

/// Громкость меню-музыки (линейная шкала 0..1).
const MENU_VOLUME: f32 = 0.45;
/// Громкость амбиента (линейная шкала 0..1).
const AMBIENT_VOLUME: f32 = 0.6;
/// Длительность плавного нарастания трека при старте, секунды.
const FADE_IN_SECS: f32 = 3.0;
/// Длительность плавного затухания при смене состояния, секунды.
const FADE_OUT_SECS: f32 = 1.5;
/// Как часто перепроверять наличие отсутствующих файлов, секунды.
const RETRY_SECS: f32 = 5.0;

// ---------------------------------------------------------------------------
// Утилиты
// ---------------------------------------------------------------------------

/// Проверка существования ассета на диске (`relative_path` - путь внутри
/// `assets/`, например `"audio/menu.mp3"`).
///
/// Используется перед каждым `AssetServer::load`, чтобы:
/// 1. не спамить ошибками загрузчика при отсутствующих файлах;
/// 2. автоматически начинать использовать файлы, как только игрок/разработчик
///    положит их в `assets/` (проверка выполняется заново при каждом запуске
///    трека и каждом срабатывании скримера).
pub fn asset_exists(relative_path: &str) -> bool {
    Path::new("assets").join(relative_path).exists()
}

/// Рабочая директория строкой для диагностики («нет звука» = не та папка).
fn current_dir_display() -> String {
    std::env::current_dir()
        .map(|d| d.display().to_string())
        .unwrap_or_else(|_| "?".to_string())
}

// ---------------------------------------------------------------------------
// Ресурс-режиссёр
// ---------------------------------------------------------------------------

/// Состояние звукорежиссёра: какие треки играют и как идут кроссфейды.
#[derive(Resource)]
struct AudioDirector {
    /// Сущность с зацикленной музыкой меню (если играет).
    menu_entity: Option<Entity>,
    /// Текущий уровень громкости меню 0..1 (для плавных нарастаний/затуханий).
    menu_level: f32,
    /// Меню-музыка сейчас гаснет (мы ушли в игру).
    menu_fading_out: bool,
    /// В консоль уже писали, что меню-музыка слышна (чтобы не спамить).
    menu_announced: bool,
    /// Сущность с текущим треком амбиента (если играет).
    ambient_entity: Option<Entity>,
    /// Индекс следующего трека амбиента в [`AMBIENT_TRACKS`].
    ambient_index: usize,
    /// Текущий уровень громкости амбиента 0..1.
    ambient_level: f32,
    /// Трек амбиента реально звучал (защита от ложного `empty()` до старта).
    ambient_heard: bool,
    /// Сколько секунд играет текущий трек амбиента.
    ambient_time: f32,
    /// Таймер повторных попыток запуска отсутствующих файлов.
    retry: Timer,
}

impl Default for AudioDirector {
    fn default() -> Self {
        Self {
            menu_entity: None,
            menu_level: 0.0,
            menu_fading_out: false,
            menu_announced: false,
            ambient_entity: None,
            ambient_index: 0,
            ambient_level: 0.0,
            ambient_heard: false,
            ambient_time: 0.0,
            retry: Timer::from_seconds(RETRY_SECS, TimerMode::Repeating),
        }
    }
}

// ---------------------------------------------------------------------------
// Плагин
// ---------------------------------------------------------------------------

/// Регистрирует звукорежиссёра.
pub struct AudioDirectorPlugin;

impl Plugin for AudioDirectorPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(AudioDirector::default())
            .add_systems(OnEnter(AppState::MainMenu), start_menu_music)
            .add_systems(OnEnter(AppState::InGame), begin_game_audio)
            .add_systems(Update, retry_menu_music.run_if(in_state(AppState::MainMenu)))
            .add_systems(Update, update_menu_music)
            .add_systems(
                Update,
                update_ambient_cycle.run_if(in_state(AppState::InGame)),
            )
            .add_systems(Update, fade_out_ambient.run_if(in_state(AppState::MainMenu)));
    }
}

// ---------------------------------------------------------------------------
// Спавн треков
// ---------------------------------------------------------------------------

/// Создать сущность с зацикленной музыкой меню (старт с нуля громкости -
/// нарастание делает [`update_menu_music`]). Возвращает `None`, если файла
/// нет на диске.
fn spawn_menu_music(commands: &mut Commands, assets: &AssetServer) -> Option<Entity> {
    if !asset_exists(MENU_MUSIC) {
        return None;
    }
    let handle: Handle<AudioSource> = assets.load(MENU_MUSIC);
    Some(
        commands
            .spawn((
                AudioPlayer::new(handle),
                PlaybackSettings::LOOP.with_volume(Volume::Linear(0.0)),
            ))
            .id(),
    )
}

/// Создать сущность с одноразовым треком амбиента (переключение треков -
/// в [`update_ambient_cycle`]). Возвращает `None`, если файла нет на диске.
fn spawn_ambient(
    commands: &mut Commands,
    assets: &AssetServer,
    index: usize,
) -> Option<Entity> {
    let path = AMBIENT_TRACKS[index % AMBIENT_TRACKS.len()];
    if !asset_exists(path) {
        return None;
    }
    let handle: Handle<AudioSource> = assets.load(path);
    Some(
        commands
            .spawn((
                AudioPlayer::new(handle),
                PlaybackSettings::ONCE.with_volume(Volume::Linear(0.0)),
            ))
            .id(),
    )
}

// ---------------------------------------------------------------------------
// Системы меню-музыки
// ---------------------------------------------------------------------------

/// Вход в меню: (пере)запуск `menu.mp3`. Если музыка ещё гасла после выхода
/// из игры - отменяем затухание и возвращаем громкость.
fn start_menu_music(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut director: ResMut<AudioDirector>,
) {
    director.menu_fading_out = false;
    director.retry.reset();
    if director.menu_entity.is_none() {
        director.menu_entity = spawn_menu_music(&mut commands, &assets);
        director.menu_level = 0.0;
        director.menu_announced = false;
        if director.menu_entity.is_some() {
            info!("Menu music started: {MENU_MUSIC}");
        } else {
            warn!(
                "{MENU_MUSIC} not found (working dir: {}) - menu will be silent (will retry automatically)",
                current_dir_display()
            );
        }
    }
}

/// В меню без музыки: периодически пробуем запуститься заново.
/// Так файл `menu.mp3`, положенный в `assets/` при запущенной игре,
/// начнёт играть сам, без перезапуска.
fn retry_menu_music(
    time: Res<Time>,
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut director: ResMut<AudioDirector>,
) {
    if director.menu_entity.is_some() || director.menu_fading_out {
        return;
    }
    director.retry.tick(time.delta());
    if director.retry.just_finished() {
        director.menu_entity = spawn_menu_music(&mut commands, &assets);
        director.menu_level = 0.0;
        director.menu_announced = false;
    }
}

/// Плавное нарастание/затухание меню-музыки через [`AudioSink`].
/// Работает в обоих состояниях: в меню - нарастание, в игре - затухание.
fn update_menu_music(
    time: Res<Time>,
    mut commands: Commands,
    mut director: ResMut<AudioDirector>,
    mut sinks: Query<&mut AudioSink>,
) {
    let Some(entity) = director.menu_entity else {
        return;
    };
    // Сина ещё нет - воспроизведение не началось (ассет грузится), ждём.
    let Ok(mut sink) = sinks.get_mut(entity) else {
        return;
    };
    let dt = time.delta_secs();
    if director.menu_fading_out {
        director.menu_level = (director.menu_level - dt / FADE_OUT_SECS).max(0.0);
    } else {
        director.menu_level = (director.menu_level + dt / FADE_IN_SECS).min(1.0);
    }
    sink.set_volume(Volume::Linear(MENU_VOLUME * director.menu_level));
    // Подтверждение в консоль, что музыка реально слышна (диагностика «нет звука»).
    if !director.menu_announced && director.menu_level >= 1.0 {
        director.menu_announced = true;
        info!("Menu music playing at full volume");
    }
    if director.menu_fading_out && director.menu_level <= 0.0 {
        commands.entity(entity).despawn();
        director.menu_entity = None;
    }
}

// ---------------------------------------------------------------------------
// Системы амбиента
// ---------------------------------------------------------------------------

/// Вход в игру: гасим меню-музыку и запускаем цикл амбиента.
fn begin_game_audio(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut director: ResMut<AudioDirector>,
) {
    // Меню-музыка не обрывается, а плавно гаснет (см. update_menu_music).
    director.menu_fading_out = true;
    director.retry.reset();
    if director.ambient_entity.is_none() {
        director.ambient_entity = spawn_ambient(&mut commands, &assets, director.ambient_index);
        director.ambient_level = 0.0;
        director.ambient_heard = false;
        director.ambient_time = 0.0;
        if director.ambient_entity.is_some() {
            info!("Ambient cycle started");
        } else {
            warn!(
                "Ambient files not found (working dir: {}) - game will be silent (will retry automatically)",
                current_dir_display()
            );
        }
    }
}

/// Цикл амбиента: нарастание текущего трека и бесшовный переход на
/// следующий, как только текущий реально закончился (`sink.empty()`).
/// Подстраивается под любую длительность файлов.
fn update_ambient_cycle(
    time: Res<Time>,
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut director: ResMut<AudioDirector>,
    mut sinks: Query<&mut AudioSink>,
) {
    let Some(entity) = director.ambient_entity else {
        // Трека нет (файлы отсутствуют) - периодически пробуем снова:
        // амбиент, положенный в assets/ при запущенной игре, подхватится сам.
        director.retry.tick(time.delta());
        if director.retry.just_finished() {
            director.ambient_entity =
                spawn_ambient(&mut commands, &assets, director.ambient_index);
            director.ambient_level = 0.0;
            director.ambient_heard = false;
            director.ambient_time = 0.0;
        }
        return;
    };
    let Ok(mut sink) = sinks.get_mut(entity) else {
        return;
    };

    let dt = time.delta_secs();
    director.ambient_time += dt;
    // Плавное нарастание в начале каждого трека.
    director.ambient_level = (director.ambient_level + dt / FADE_IN_SECS).min(1.0);
    sink.set_volume(Volume::Linear(AMBIENT_VOLUME * director.ambient_level));

    if !sink.empty() {
        director.ambient_heard = true;
    }
    // Трек закончился - переходим на следующий из пары (1 -> 2 -> 1 -> ...).
    if director.ambient_heard && sink.empty() && director.ambient_time > 2.0 {
        commands.entity(entity).despawn();
        director.ambient_index = (director.ambient_index + 1) % AMBIENT_TRACKS.len();
        director.ambient_entity = spawn_ambient(&mut commands, &assets, director.ambient_index);
        director.ambient_level = 0.0;
        director.ambient_heard = false;
        director.ambient_time = 0.0;
    }
}

/// Возврат в меню: плавно гасим амбиент, затем удаляем сущность.
fn fade_out_ambient(
    time: Res<Time>,
    mut commands: Commands,
    mut director: ResMut<AudioDirector>,
    mut sinks: Query<&mut AudioSink>,
) {
    let Some(entity) = director.ambient_entity else {
        return;
    };
    let Ok(mut sink) = sinks.get_mut(entity) else {
        return;
    };
    director.ambient_level =
        (director.ambient_level - time.delta_secs() / FADE_OUT_SECS).max(0.0);
    sink.set_volume(Volume::Linear(AMBIENT_VOLUME * director.ambient_level));
    if director.ambient_level <= 0.0 {
        commands.entity(entity).despawn();
        director.ambient_entity = None;
    }
}
