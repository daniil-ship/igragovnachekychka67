//! «Эхо Забытых Стен» - психологический хоррор от первого лица.
//!
//! Точка входа: окно, глобальные состояния [`GameState`], подключение плагинов,
//! главное меню, три акта, HUD и экраны финалов.
//!
//! Архитектура (ECS, по модулям):
//! - [`player`] - игрок: движение, стамина, принудительный бег, фонарик, HUD;
//! - [`audio`] - звукорежиссёр: музыка меню и циклический амбиент;
//! - [`hallucinations`] - безумие и скримеры (PNG на весь экран + MP3).
//! - [`atmosphere`] - лампы, туман, пыль, виньетка и зерно плёнки.
//! - [`interaction`] - батарейки, кассеты, субтитры (Акт 2).
//! - [`shadow`] - 3D-тень-скример и Тень-сталкер (монстр Актов 2-3).

mod atmosphere;
mod audio;
mod hallucinations;
mod interaction;
mod player;
mod shadow;
mod textures;

use std::path::PathBuf;

use atmosphere::AtmospherePlugin;
use audio::AudioDirectorPlugin;
use bevy::prelude::*;
use bevy::state::state_scoped::StateScoped;
use bevy::window::{CursorGrabMode, WindowPlugin, WindowResolution};
use hallucinations::{HallucinationsPlugin, Insanity, ScreamerState};
use interaction::{AudioCassette, BatteryItem, GeneratorLever, InteractionPlugin, QuestItem};
use player::{
    Flashlight, ForcedRun, InsanityVignette, Player, PlayerPlugin, Stamina, StaminaFill,
};
use rand::seq::SliceRandom;
use rand::Rng;
use shadow::ShadowPlugin;

// ---------------------------------------------------------------------------
// Константы мира
// ---------------------------------------------------------------------------

/// Размер одной клетки лабиринта в метрах.
pub const CELL: f32 = 4.0;
/// Высота стен лабиринта.
pub const WALL_H: f32 = 3.2;
/// Размер лабиринта в клетках (только НЕЧЁТНЫЕ числа - требование генератора).
const MAZE_W: usize = 21;
const MAZE_H: usize = 15;
/// Сколько фрагментов эха нужно собрать для перехода из Акта 2 в Акт 3.
pub const FRAGMENT_COUNT: usize = 5;
/// Дистанция сбора фрагмента (по горизонтали), метры.
const PICKUP_RADIUS: f32 = 1.5;
/// Сколько секунд даёт таймер реактора в Акте 3 (3 минуты до теплового взрыва).
const REACTOR_SECS: f32 = 180.0;
/// Радиус триггера лифта (Акт 3), метры.
const ELEVATOR_RADIUS: f32 = 2.2;
/// PNG-текстуры заброшки (пути внутри `assets/`). Если файла нет -
/// используется процедурная текстура из [`textures`].
pub const TEX_WALL_A: &str = "textures/wall1.png";
pub const TEX_WALL_B: &str = "textures/wall2.png";
pub const TEX_FLOOR: &str = "textures/floor.png";
pub const TEX_CEIL: &str = "textures/ceil.png";

// ---------------------------------------------------------------------------
// Состояния, ресурсы, компоненты уровня
// ---------------------------------------------------------------------------

/// Глобальные состояния игры: меню, три акта, два финала.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GameState {
    /// Главное меню (музыка `menu.mp3`, фильм-нуар).
    #[default]
    MainMenu,
    /// Акт 1: спуск в Сектор Г, генератор, освоение.
    Act1_TheDescent,
    /// Акт 2: тьма, безумие, фрагменты, кассеты, Тень рядом.
    Act2_TheInsanity,
    /// Акт 3: реактор, 3 минуты, охота, лифт.
    Act3_TheReactor,
    /// Поражение: сердце, таймер или Поглощение.
    GameOver,
    /// Победа: Искупление.
    GameWon,
}

/// Ось-выровненный бокс стены в плоскости XZ (для коллизий игрока).
#[derive(Debug, Clone, Copy)]
pub struct WallAabb {
    pub min_x: f32,
    pub max_x: f32,
    pub min_z: f32,
    pub max_z: f32,
}

/// Все коллайдеры стен текущего уровня. Заполняется в [`generate_level`].
#[derive(Resource, Default)]
pub struct LevelColliders {
    pub walls: Vec<WallAabb>,
}

/// Прогресс забега: фрагменты Акта 2, реликвии брата и финал.
#[derive(Resource)]
pub struct GameProgress {
    pub collected: u32,
    pub total: u32,
    /// Найденные личные вещи Миши (макс 5 - условие хорошей концовки).
    pub quest_items: u32,
    /// Чем закончился забег (заполняется при переходе в GameOver).
    pub ending: Option<EndingKind>,
}

impl Default for GameProgress {
    fn default() -> Self {
        Self {
            collected: 0,
            total: FRAGMENT_COUNT as u32,
            quest_items: 0,
            ending: None,
        }
    }
}

/// Чем закончился забег (вариант экрана GameOver).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndingKind {
    /// Сердце не выдержало (25% при истощении стамины).
    HeartDeath,
    /// Реактор взорвался раньше, чем герой дошёл до лифта.
    Timeout,
    /// Поглощение: в лифте без 5 реликвий.
    Absorbed,
}

/// Отложенный переход между актами (рычаг/фрагменты дают пару секунд передышки).
#[derive(Resource)]
pub struct PendingTransition {
    pub timer: Timer,
    pub next: GameState,
}

/// Таймер реактора Акта 3: секунды до теплового взрыва.
#[derive(Resource)]
pub struct ReactorTimer {
    pub time_left: f32,
}

/// Настройки из меню (живут всю сессию, между состояниями не сбрасываются).
#[derive(Resource)]
pub struct GameSettings {
    /// Зерно плёнки поверх картинки.
    pub film_grain: bool,
    /// Тёмная виньетка по краям экрана.
    pub vignette: bool,
}

impl Default for GameSettings {
    fn default() -> Self {
        Self {
            film_grain: true,
            vignette: true,
        }
    }
}

/// Светящийся фрагмент эха - цель Акта 2 (5 штук в дальних клетках лабиринта).
#[derive(Component)]
struct EchoFragment;

/// Лифт - цель Акта 3 (триггер финала).
#[derive(Component)]
struct Elevator;

/// Маркер заливки тонкой красной полоски таймера реактора (HUD Акта 3).
#[derive(Component)]
struct MeltdownFill;

/// Маркер мигающей подсказки в меню.
#[derive(Component)]
struct MenuPrompt;

/// Действие кнопки настроек в меню.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
enum SettingsAction {
    ToggleGrain,
    ToggleVignette,
}

/// Подпись кнопки настроек (висит на тексте, ребёнке кнопки).
#[derive(Component)]
struct SettingsLabel(SettingsAction);

// ---------------------------------------------------------------------------
// Точка входа
// ---------------------------------------------------------------------------

fn main() {
    // Сначала - гарантируем видимость assets/ (иначе нет музыки и PNG).
    ensure_asset_root();
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Echo of Forgotten Walls".to_string(),
                resolution: WindowResolution::new(1280.0, 720.0),
                ..default()
            }),
            ..default()
        }))
        // Почти чёрный фон + ресурсы уровня по умолчанию.
        .insert_resource(ClearColor(Color::srgb(0.005, 0.005, 0.01)))
        .insert_resource(LevelColliders::default())
        .insert_resource(GameProgress::default())
        .insert_resource(GameSettings::default())
        .init_state::<GameState>()
        // В Bevy 0.16 state-scoped сущности включаются явно,
        // иначе маркеры StateScoped не будут ничего удалять.
        .enable_state_scoped_entities::<GameState>()
        .add_plugins((
            PlayerPlugin,
            AudioDirectorPlugin,
            AtmospherePlugin,
            HallucinationsPlugin,
            InteractionPlugin,
            ShadowPlugin,
        ))
        // Холодный тусклый свет окружения.
        .add_systems(Startup, setup_ambient_light)
        // Главное меню.
        .add_systems(OnEnter(GameState::MainMenu), setup_menu)
        .add_systems(
            Update,
            (menu_input, blink_menu_prompt, settings_buttons).run_if(in_state(GameState::MainMenu)),
        )
        // Акты: сброс забега (только Акт 1), генерация, HUD, курсор.
        .add_systems(
            OnEnter(GameState::Act1_TheDescent),
            (reset_run, generate_level, setup_hud, grab_cursor),
        )
        .add_systems(
            OnEnter(GameState::Act2_TheInsanity),
            (generate_level, setup_hud, grab_cursor),
        )
        .add_systems(
            OnEnter(GameState::Act3_TheReactor),
            (reset_reactor, generate_level, setup_hud, grab_cursor),
        )
        // Игровые системы актов.
        .add_systems(
            Update,
            (
                collect_fragments,
                tick_transition,
                tick_reactor,
                elevator_trigger,
                escape_to_menu,
                restart_run,
                bob_fragments,
            )
                .run_if(in_act),
        )
        // Финалы: экран, курсор, ввод. Потеря фокуса тоже отпускает мышь.
        .add_systems(OnEnter(GameState::GameOver), (setup_end_screen, release_cursor))
        .add_systems(OnEnter(GameState::GameWon), (setup_end_screen, release_cursor))
        .add_systems(Update, (escape_to_menu, restart_run).run_if(in_end))
        .add_systems(OnExit(GameState::Act1_TheDescent), release_cursor)
        .add_systems(OnExit(GameState::Act2_TheInsanity), release_cursor)
        .add_systems(OnExit(GameState::Act3_TheReactor), release_cursor)
        .run();
}

/// Гарантия, что папка `assets/` видна игре.
///
/// Если её нет рядом с рабочей директорией (запуск не из корня проекта),
/// пробуем перейти в папку с .exe или вверх по дереву. Иначе игра молча
/// останется без музыки, звуков и PNG (все ассеты опциональны с тихими
/// заглушками) - самая частая причина жалоб «нет звука в меню».
/// Вызывается первой строкой main(), ещё до создания App.
fn ensure_asset_root() {
    if PathBuf::from("assets").is_dir() {
        return;
    }
    // Рядом нет - кандидаты: папка с .exe и родители рабочей директории.
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.to_path_buf());
        }
    }
    let mut dir = std::env::current_dir().unwrap_or_default();
    for _ in 0..5 {
        if !dir.pop() {
            break;
        }
        candidates.push(dir.clone());
    }
    for candidate in candidates {
        if candidate.join("assets").is_dir() {
            if std::env::set_current_dir(&candidate).is_ok() {
                // Логгера Bevy ещё нет (App не создан) - пишем в stdout напрямую.
                println!(
                    "No assets/ next to working dir - switched to {}",
                    candidate.display()
                );
                return;
            }
        }
    }
    eprintln!(
        "WARNING: assets/ not found (working dir: {}). Music, screamer sounds and PNG will be missing - run the game from the project root!",
        std::env::current_dir()
            .map(|d| d.display().to_string())
            .unwrap_or_else(|_| "?".to_string())
    );
}

/// Run-условие «идёт игровой акт» (любой из трёх).
pub fn in_act(state: Res<State<GameState>>) -> bool {
    matches!(
        state.get(),
        GameState::Act1_TheDescent | GameState::Act2_TheInsanity | GameState::Act3_TheReactor
    )
}

/// Run-условие «экран финала» (поражение или победа).
pub fn in_end(state: Res<State<GameState>>) -> bool {
    matches!(state.get(), GameState::GameOver | GameState::GameWon)
}

/// Run-условие «управление игрока разрешено»: идёт акт и секунда скримера
/// не блокирует ввод. Состояния-финалы управление выключают сами.
pub fn game_input_allowed(state: Res<State<GameState>>, screamer: Res<ScreamerState>) -> bool {
    matches!(
        state.get(),
        GameState::Act1_TheDescent | GameState::Act2_TheInsanity | GameState::Act3_TheReactor
    ) && !screamer.active
}

// ---------------------------------------------------------------------------
// Свет окружения
// ---------------------------------------------------------------------------

/// Едва заметный холодный свет, чтобы тьма не была абсолютно чёрной.
/// (Ресурс уже создан `PbrPlugin`, мы лишь приглушаем его.)
/// Основной свет теперь дают потолочные лампы (см. [`atmosphere`]).
fn setup_ambient_light(mut ambient_light: ResMut<AmbientLight>) {
    ambient_light.color = Color::srgb(0.5, 0.58, 0.75);
    ambient_light.brightness = 0.09;
}

// ---------------------------------------------------------------------------
// Главное меню
// ---------------------------------------------------------------------------

/// Построение меню: 2D-камера + заголовок, подсказка и управление.
fn setup_menu(
    mut commands: Commands,
    mut windows: Query<&mut Window>,
    settings: Res<GameSettings>,
) {
    // В меню курсор всегда видим и свободен.
    for mut window in &mut windows {
        window.cursor_options.grab_mode = CursorGrabMode::None;
        window.cursor_options.visible = true;
    }

    commands.spawn((Camera2d, StateScoped(GameState::MainMenu)));

    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(18.0),
                ..default()
            },
            BackgroundColor(Color::srgb(0.012, 0.004, 0.006)),
            StateScoped(GameState::MainMenu),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("ECHO OF FORGOTTEN WALLS"),
                TextFont {
                    font_size: 60.0,
                    ..default()
                },
                TextColor(Color::srgb(0.72, 0.08, 0.08)),
                StateScoped(GameState::MainMenu),
            ));
            parent.spawn((
                Text::new("A PSYCHOLOGICAL HORROR"),
                TextFont {
                    font_size: 24.0,
                    ..default()
                },
                TextColor(Color::srgb(0.45, 0.42, 0.45)),
                StateScoped(GameState::MainMenu),
            ));
            parent.spawn((
                Text::new("ARTHUR, 34, ARCHIVIST - ARCHIVE-404, FINAL INVENTORY"),
                TextFont {
                    font_size: 16.0,
                    ..default()
                },
                TextColor(Color::srgb(0.55, 0.55, 0.58)),
                StateScoped(GameState::MainMenu),
            ));
            parent.spawn((
                Text::new("PRESS ENTER OR CLICK TO BEGIN"),
                TextFont {
                    font_size: 28.0,
                    ..default()
                },
                TextColor(Color::srgb(0.85, 0.8, 0.78)),
                MenuPrompt,
                StateScoped(GameState::MainMenu),
            ));
            parent.spawn((
                Text::new(
                    "WASD / ARROWS - MOVE      MOUSE - LOOK      SHIFT - RUN\n\
                     F - FLASHLIGHT (DRAINS BATTERY)      E - USE      R - RESTART RUN\n\
                     ACT 1: PULL THE GENERATOR LEVER - ACT 2: 5 ECHO FRAGMENTS - ACT 3: REACH THE LIFT\n\
                     DARKNESS FEEDS MADNESS - BATTERIES SAVE LIGHT - FIND 5 KEEPSAKES FOR THE GOOD END",
                ),
                TextFont {
                    font_size: 18.0,
                    ..default()
                },
                TextColor(Color::srgb(0.5, 0.48, 0.5)),
                StateScoped(GameState::MainMenu),
            ));
            parent.spawn((
                Text::new("HEADPHONES RECOMMENDED"),
                TextFont {
                    font_size: 16.0,
                    ..default()
                },
                TextColor(Color::srgb(0.5, 0.12, 0.12)),
                StateScoped(GameState::MainMenu),
            ));
            // Настройки экрана: кнопки-переключатели (клик по ним не стартует игру).
            parent.spawn((
                Text::new("- SETTINGS -"),
                TextFont {
                    font_size: 20.0,
                    ..default()
                },
                TextColor(Color::srgb(0.6, 0.55, 0.55)),
                StateScoped(GameState::MainMenu),
            ));
            for action in [SettingsAction::ToggleGrain, SettingsAction::ToggleVignette] {
                parent
                    .spawn((
                        Button,
                        Interaction::None,
                        Node {
                            padding: UiRect {
                                left: Val::Px(28.0),
                                right: Val::Px(28.0),
                                top: Val::Px(10.0),
                                bottom: Val::Px(10.0),
                            },
                            justify_content: JustifyContent::Center,
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.10, 0.05, 0.06)),
                        action,
                        StateScoped(GameState::MainMenu),
                    ))
                    .with_children(|button| {
                        button.spawn((
                            Text::new(setting_label(action, &settings)),
                            TextFont {
                                font_size: 20.0,
                                ..default()
                            },
                            TextColor(Color::srgb(0.85, 0.8, 0.78)),
                            SettingsLabel(action),
                            StateScoped(GameState::MainMenu),
                        ));
                    });
            }
        });
}

/// Вход в игру по Enter/Space/клику (клик по кнопке настройки не стартует).
fn menu_input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    buttons: Query<&Interaction, With<Button>>,
    mut next_state: ResMut<NextState<GameState>>,
) {
    if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space) {
        next_state.set(GameState::Act1_TheDescent);
        return;
    }
    if mouse.just_pressed(MouseButton::Left) {
        let on_button = buttons
            .iter()
            .any(|i| *i == Interaction::Hovered || *i == Interaction::Pressed);
        if !on_button {
            next_state.set(GameState::Act1_TheDescent);
        }
    }
}

/// Мигание подсказки «PRESS ENTER...» (без таймера - по времени кадра).
fn blink_menu_prompt(time: Res<Time>, mut query: Query<&mut Text, With<MenuPrompt>>) {
    let visible = (time.elapsed_secs() * 1.2).fract() < 0.6;
    for mut text in &mut query {
        let message = if visible {
            "PRESS ENTER OR CLICK TO BEGIN"
        } else {
            ""
        };
        if text.0.as_str() != message {
            text.0 = message.to_string();
        }
    }
}

/// Кнопки настроек в меню: подсветка при наведении + переключение по клику.
fn settings_buttons(
    mut settings: ResMut<GameSettings>,
    mut buttons: Query<
        (&Interaction, &SettingsAction, &mut BackgroundColor),
        (Changed<Interaction>, With<Button>),
    >,
    mut labels: Query<(&SettingsLabel, &mut Text)>,
) {
    for (interaction, action, mut bg) in &mut buttons {
        match *interaction {
            Interaction::Pressed => {
                match action {
                    SettingsAction::ToggleGrain => {
                        settings.film_grain = !settings.film_grain;
                    }
                    SettingsAction::ToggleVignette => {
                        settings.vignette = !settings.vignette;
                    }
                }
                for (label, mut text) in &mut labels {
                    text.0 = setting_label(label.0, &settings);
                }
                bg.0 = Color::srgb(0.30, 0.12, 0.12);
            }
            Interaction::Hovered => {
                bg.0 = Color::srgb(0.20, 0.09, 0.10);
            }
            Interaction::None => {
                bg.0 = Color::srgb(0.10, 0.05, 0.06);
            }
        }
    }
}

/// Подпись кнопки настройки: имя + текущее состояние.
fn setting_label(action: SettingsAction, settings: &GameSettings) -> String {
    let (name, on) = match action {
        SettingsAction::ToggleGrain => ("FILM GRAIN", settings.film_grain),
        SettingsAction::ToggleVignette => ("VIGNETTE", settings.vignette),
    };
    format!("{name}: {}", if on { "ON" } else { "OFF" })
}

// ---------------------------------------------------------------------------
// Генерация уровня
// ---------------------------------------------------------------------------

/// Центр клетки лабиринта в мировых координатах (x, z). Карта центрирована
/// в начале координат.
pub(crate) fn cell_center(cx: usize, cy: usize) -> (f32, f32) {
    (
        (cx as f32 - MAZE_W as f32 / 2.0 + 0.5) * CELL,
        (cy as f32 - MAZE_H as f32 / 2.0 + 0.5) * CELL,
    )
}

/// Генератор лабиринта: итеративный backtracker (`true` - стена).
/// После построения часть тупиков «заплетается» в петли, чтобы были обходы.
fn generate_maze(w: usize, h: usize) -> Vec<Vec<bool>> {
    debug_assert!(w % 2 == 1 && h % 2 == 1, "maze size must be odd");
    let mut rng = rand::thread_rng();
    let mut wall = vec![vec![true; w]; h];
    let mut stack = vec![(1usize, 1usize)];
    wall[1][1] = false;

    const DIRS_2: [(isize, isize); 4] = [(2, 0), (-2, 0), (0, 2), (0, -2)];
    while let Some(&(cx, cy)) = stack.last() {
        let mut options = Vec::with_capacity(4);
        for (dx, dy) in DIRS_2 {
            let nx = cx as isize + dx;
            let ny = cy as isize + dy;
            if nx > 0 && ny > 0 && nx < w as isize - 1 && ny < h as isize - 1
                && wall[ny as usize][nx as usize]
            {
                options.push((nx as usize, ny as usize));
            }
        }
        match options.choose(&mut rng) {
            Some(&(nx, ny)) => {
                wall[(cy + ny) / 2][(cx + nx) / 2] = false;
                wall[ny][nx] = false;
                stack.push((nx, ny));
            }
            None => {
                stack.pop();
            }
        }
    }

    // Заплетаем ~35% тупиков: сносим одну стену - появляются петли.
    const DIRS_1: [(isize, isize); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    for cy in (1..h - 1).step_by(2) {
        for cx in (1..w - 1).step_by(2) {
            let open = DIRS_1
                .iter()
                .filter(|(dx, dy)| {
                    !wall[(cy as isize + dy) as usize][(cx as isize + dx) as usize]
                })
                .count();
            if open == 1 && rng.gen_bool(0.35) {
                let closed: Vec<(isize, isize)> = DIRS_1
                    .into_iter()
                    .filter(|(dx, dy)| {
                        let nx = cx as isize + dx;
                        let ny = cy as isize + dy;
                        nx > 0 && ny > 0
                            && nx < w as isize - 1
                            && ny < h as isize - 1
                            && wall[ny as usize][nx as usize]
                    })
                    .collect();
                if let Some((dx, dy)) = closed.choose(&mut rng) {
                    wall[(cy as isize + dy) as usize][(cx as isize + dx) as usize] = false;
                }
            }
        }
    }
    wall
}

/// PNG-текстура из assets или процедурная заглушка, если файла нет на диске.
fn texture_or_fallback(
    assets: &AssetServer,
    images: &mut ResMut<Assets<Image>>,
    path: &str,
    fallback: fn() -> Image,
) -> Handle<Image> {
    if crate::audio::asset_exists(path) {
        assets.load(path)
    } else {
        info!("{path} not found - using procedural fallback texture");
        images.add(fallback())
    }
}

/// Построение уровня при входе в акт: пол, потолок, стены, игрок с
/// фонариком и содержимое по акту (рычаг, фрагменты, кассеты, лифт,
/// реликвии, Тень). Каждый акт - новый случайный лабиринт.
fn generate_level(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    assets: Res<AssetServer>,
    mut colliders: ResMut<LevelColliders>,
    state: Res<State<GameState>>,
) {
    // Сброс коллайдеров (прогресс забега сбрасывает `reset_run` на входе в Акт 1).
    colliders.walls.clear();
    let act = *state.get();

    let maze = generate_maze(MAZE_W, MAZE_H);

    // Текстуры заброшки: PNG из assets (или процедурные, если файлов нет).
    let wall_tex_a =
        texture_or_fallback(&assets, &mut images, TEX_WALL_A, textures::build_wall_texture);
    let wall_tex_b =
        texture_or_fallback(&assets, &mut images, TEX_WALL_B, textures::build_wall_texture);
    let floor_tex =
        texture_or_fallback(&assets, &mut images, TEX_FLOOR, textures::build_floor_texture);
    let ceil_tex =
        texture_or_fallback(&assets, &mut images, TEX_CEIL, textures::build_ceiling_texture);

    // Общие материалы. base_color умножается на текстуру и работает оттенком.
    let floor_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 1.0, 1.0),
        base_color_texture: Some(floor_tex),
        // Полусухой грязный пол: бликует под лампами вместо матовой каши.
        perceptual_roughness: 0.45,
        ..default()
    });
    let ceil_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 1.0, 1.0),
        base_color_texture: Some(ceil_tex),
        perceptual_roughness: 0.95,
        ..default()
    });
    let wall_mat_a = materials.add(StandardMaterial {
        base_color: Color::srgb(0.85, 0.83, 0.88),
        base_color_texture: Some(wall_tex_a),
        perceptual_roughness: 0.9,
        ..default()
    });
    let wall_mat_b = materials.add(StandardMaterial {
        base_color: Color::srgb(0.6, 0.58, 0.63),
        base_color_texture: Some(wall_tex_b),
        perceptual_roughness: 0.9,
        ..default()
    });

    // Стены: по кубу на каждую клетку-стену + коллайдер.
    // Пол и потолок: по плите на каждую клетку пола, чтобы текстура не тянулась.
    for (cy, row) in maze.iter().enumerate() {
        for (cx, &is_wall) in row.iter().enumerate() {
            let (x, z) = cell_center(cx, cy);
            if !is_wall {
                commands.spawn((
                    Mesh3d(meshes.add(Cuboid::new(CELL, 0.2, CELL))),
                    MeshMaterial3d(floor_mat.clone()),
                    Transform::from_xyz(x, -0.1, z),
                    StateScoped(act),
                ));
                commands.spawn((
                    Mesh3d(meshes.add(Cuboid::new(CELL, 0.2, CELL))),
                    MeshMaterial3d(ceil_mat.clone()),
                    Transform::from_xyz(x, WALL_H + 0.1, z),
                    StateScoped(act),
                ));
                continue;
            }
            let mat = if (cx + cy) % 2 == 0 {
                wall_mat_a.clone()
            } else {
                wall_mat_b.clone()
            };
            commands.spawn((
                Mesh3d(meshes.add(Cuboid::new(CELL, WALL_H, CELL))),
                MeshMaterial3d(mat),
                Transform::from_xyz(x, WALL_H / 2.0, z),
                StateScoped(act),
            ));
            colliders.walls.push(WallAabb {
                min_x: x - CELL / 2.0,
                max_x: x + CELL / 2.0,
                min_z: z - CELL / 2.0,
                max_z: z + CELL / 2.0,
            });
        }
    }

    // Потолочные лампы, мигающие и разбитые плафоны, тёмные зоны.
    atmosphere::spawn_lamps(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut images,
        &maze,
        act,
    );

    // Игрок в клетке (1, 1): смотрим в открытый проход -
    // сначала пробуем восток, иначе юг (один из них точно открыт).
    let (spawn_x, spawn_z) = cell_center(1, 1);
    let yaw = if !maze[1][2] {
        -std::f32::consts::FRAC_PI_2 // восток (+X)
    } else {
        std::f32::consts::PI // юг (+Z)
    };
    commands
        .spawn((
            Camera3d::default(),
            // Густой чёрный туман: дальние коридоры тонут во тьме.
            DistanceFog {
                color: Color::srgb(0.008, 0.009, 0.014),
                falloff: FogFalloff::Exponential {
                    density: atmosphere::FOG_DENSITY,
                },
                ..default()
            },
            Transform {
                translation: Vec3::new(spawn_x, player::EYE_HEIGHT, spawn_z),
                rotation: Quat::from_euler(EulerRot::YXZ, yaw, 0.0, 0.0),
                ..default()
            },
            Player { yaw, pitch: 0.0 },
            Stamina::default(),
            ForcedRun(false),
            Insanity(0.0),
            StateScoped(act),
        ))
        .with_children(|parent| {
            // Фонарик - спотлайт, ребёнок камеры (светит туда же, куда смотрим).
            parent.spawn((
                SpotLight {
                    color: Color::srgb(1.0, 0.95, 0.85),
                    intensity: player::FLASHLIGHT_INTENSITY,
                    range: player::FLASHLIGHT_RANGE,
                    inner_angle: 0.22,
                    outer_angle: player::FLASHLIGHT_OUTER_ANGLE,
                    shadows_enabled: true,
                    ..default()
                },
                Transform::from_xyz(0.15, -0.2, 0.0),
                Flashlight {
                    base_intensity: player::FLASHLIGHT_INTENSITY,
                    is_on: true,
                    battery: Flashlight::MAX_BATTERY,
                },
                StateScoped(act),
            ));
        });

    // --- Содержимое актов: батарейки в каждом, остальное - по акту ---
    let mut rng = rand::thread_rng();
    let mut floor_cells: Vec<(usize, usize)> = Vec::new();
    for (cy, row) in maze.iter().enumerate() {
        for (cx, &is_wall) in row.iter().enumerate() {
            if !is_wall {
                floor_cells.push((cx, cy));
            }
        }
    }
    // Батарейки есть в каждом акте (в третьем - последние крохи).
    let battery_count = match act {
        GameState::Act3_TheReactor => 4,
        _ => interaction::BATTERY_COUNT,
    };
    for _ in 0..battery_count {
        if let Some(&(cx, cy)) = floor_cells.choose(&mut rng) {
            let (x, z) = cell_center(cx, cy);
            spawn_battery(
                &mut commands,
                &mut meshes,
                &mut materials,
                Vec3::new(x, 0.0, z),
                act,
            );
        }
    }
    match act {
        GameState::Act1_TheDescent => {
            // Рычаг генератора в глубине сектора + две первые реликвии брата.
            if let Some(spot) = random_floor_spot(&mut rng, &floor_cells, 6) {
                spawn_lever(&mut commands, &mut meshes, &mut materials, spot, act);
            }
            for index in 0..2 {
                if let Some(spot) = random_floor_spot(&mut rng, &floor_cells, 0) {
                    spawn_quest_item(
                        &mut commands,
                        &mut meshes,
                        &mut materials,
                        spot,
                        index,
                        act,
                    );
                }
            }
            interaction::spawn_subtitle(
                &mut commands,
                "ACT 1 - THE DESCENT",
                "Sector G. The blast door seals behind you. Find the generator lever and get the lights back.",
                7.0,
                act,
            );
        }
        GameState::Act2_TheInsanity => {
            // 5 фрагментов эха в дальних от спавна клетках.
            let mut candidates: Vec<(usize, usize)> = floor_cells
                .iter()
                .copied()
                .filter(|&(cx, cy)| cx.abs_diff(1) + cy.abs_diff(1) >= 14)
                .collect();
            if candidates.len() < FRAGMENT_COUNT {
                // Лабиринт тесный - fallback: просто самые дальние клетки пола.
                let mut all = floor_cells.clone();
                all.sort_by_key(|&(cx, cy)| {
                    std::cmp::Reverse(cx.abs_diff(1) + cy.abs_diff(1))
                });
                candidates = all.into_iter().take(FRAGMENT_COUNT).collect();
            }
            candidates.shuffle(&mut rng);
            for &(cx, cy) in candidates.iter().take(FRAGMENT_COUNT) {
                let (x, z) = cell_center(cx, cy);
                spawn_fragment(
                    &mut commands,
                    &mut meshes,
                    &mut materials,
                    Vec3::new(x, 0.0, z),
                    act,
                );
            }
            // 3 кассеты с записями профессора - подальше от входа.
            for lore in 0..interaction::CASSETTE_COUNT {
                if let Some(spot) = random_floor_spot(&mut rng, &floor_cells, 8) {
                    spawn_cassette(
                        &mut commands,
                        &mut meshes,
                        &mut materials,
                        spot,
                        lore,
                        act,
                    );
                }
            }
            // Реликвии 3-4 и Тень на патрулировании.
            for index in 2..4 {
                if let Some(spot) = random_floor_spot(&mut rng, &floor_cells, 0) {
                    spawn_quest_item(
                        &mut commands,
                        &mut meshes,
                        &mut materials,
                        spot,
                        index,
                        act,
                    );
                }
            }
            if let Some(spot) = random_floor_spot(&mut rng, &floor_cells, 10) {
                shadow::spawn_monster(&mut commands, &mut meshes, &mut materials, spot, act);
            }
            interaction::spawn_subtitle(
                &mut commands,
                "ACT 2 - REJECTION",
                "Only the red emergency light remains. Collect 5 echo fragments. Do not let your torch die.",
                7.0,
                act,
            );
        }
        GameState::Act3_TheReactor => {
            // Лифт - в самой дальней от спавна клетке.
            if let Some(&(cx, cy)) = floor_cells
                .iter()
                .max_by_key(|&&(cx, cy)| cx.abs_diff(1) + cy.abs_diff(1))
            {
                let (x, z) = cell_center(cx, cy);
                spawn_elevator(
                    &mut commands,
                    &mut meshes,
                    &mut materials,
                    Vec3::new(x, 0.0, z),
                    act,
                );
            }
            // Последняя реликвия и Тень на охоте.
            if let Some(spot) = random_floor_spot(&mut rng, &floor_cells, 0) {
                spawn_quest_item(&mut commands, &mut meshes, &mut materials, spot, 4, act);
            }
            if let Some(spot) = random_floor_spot(&mut rng, &floor_cells, 10) {
                shadow::spawn_monster(&mut commands, &mut meshes, &mut materials, spot, act);
            }
            interaction::spawn_subtitle(
                &mut commands,
                "ACT 3 - THE REACTOR",
                "Three minutes to meltdown. All lights are dead. Reach the lift - and hold your light on the shadow.",
                8.0,
                act,
            );
        }
        _ => {}
    }

    info!("Level generated for {act:?}: {} walls", colliders.walls.len());
}

/// Спавн одного фрагмента эха: светящаяся сфера + точечный свет.
fn spawn_fragment(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    spot: Vec3,
    act: GameState,
) {
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(0.28).mesh().uv(24, 16))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.45, 0.9, 1.0),
            unlit: true,
            ..default()
        })),
        Transform::from_translation(spot + Vec3::new(0.0, 1.2, 0.0)),
        PointLight {
            color: Color::srgb(0.45, 0.85, 1.0),
            intensity: 200_000.0,
            range: 7.0,
            ..default()
        },
        EchoFragment,
        StateScoped(act),
    ));
}

/// Спавн одной батарейки: светящийся зелёный брусок (Акт 2).
fn spawn_battery(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    spot: Vec3,
    act: GameState,
) {
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(0.14, 0.22, 0.14))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.2, 0.9, 0.3),
            unlit: true,
            ..default()
        })),
        Transform::from_translation(spot + Vec3::new(0.0, 0.35, 0.0)),
        BatteryItem {
            recharge_amount: interaction::BATTERY_RECHARGE,
        },
        StateScoped(act),
    ));
}

/// Спавн одной сюжетной кассеты: тёмно-красный брусок (Акт 2).
/// `lore_index` выбирает аудиофайл и текст субтитров.
fn spawn_cassette(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    spot: Vec3,
    lore_index: usize,
    act: GameState,
) {
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(0.26, 0.05, 0.16))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.5, 0.06, 0.07),
            unlit: true,
            ..default()
        })),
        Transform::from_translation(spot + Vec3::new(0.0, 0.3, 0.0)),
        AudioCassette {
            audio_path: interaction::CASSETTE_TRACKS
                [lore_index % interaction::CASSETTE_TRACKS.len()]
            .to_string(),
            was_played: false,
            lore_index,
        },
        StateScoped(act),
    ));
}

// ---------------------------------------------------------------------------
// Спавн объектов актов: рычаг, реликвии, лифт
// ---------------------------------------------------------------------------

/// Случайная клетка пола не ближе `min_dist` (по Манхэттену от спавна (1, 1)).
/// Если подходящих нет - любая клетка пола.
fn random_floor_spot(
    rng: &mut rand::rngs::ThreadRng,
    floor: &[(usize, usize)],
    min_dist: usize,
) -> Option<Vec3> {
    let pool: Vec<(usize, usize)> = floor
        .iter()
        .copied()
        .filter(|&(cx, cy)| cx.abs_diff(1) + cy.abs_diff(1) >= min_dist)
        .collect();
    let source: &[(usize, usize)] = if pool.is_empty() { floor } else { &pool };
    source.choose(rng).map(|&(cx, cy)| {
        let (x, z) = cell_center(cx, cy);
        Vec3::new(x, 0.0, z)
    })
}

/// Спавн рычага генератора (Акт 1): красный брусок на уровне пояса.
fn spawn_lever(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    spot: Vec3,
    act: GameState,
) {
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(0.3, 0.5, 0.3))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.7, 0.08, 0.06),
            unlit: true,
            ..default()
        })),
        Transform::from_translation(spot + Vec3::new(0.0, 0.55, 0.0)),
        GeneratorLever { pulled: false },
        StateScoped(act),
    ));
}

/// Спавн реликвии брата (золотая сфера, `index` 0..5 - какая вещь).
fn spawn_quest_item(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    spot: Vec3,
    index: usize,
    act: GameState,
) {
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(0.12).mesh().uv(16, 12))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.95, 0.75, 0.3),
            unlit: true,
            ..default()
        })),
        Transform::from_translation(spot + Vec3::new(0.0, 0.4, 0.0)),
        QuestItem { index },
        StateScoped(act),
    ));
}

/// Спавн лифта (Акт 3): светящийся портал с ярким светом-маяком.
fn spawn_elevator(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    spot: Vec3,
    act: GameState,
) {
    // Задняя светящаяся панель.
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(2.4, 3.0, 0.3))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.92, 0.95, 1.0),
            unlit: true,
            ..default()
        })),
        Transform::from_translation(spot + Vec3::new(0.0, 1.5, 0.0)),
        Elevator,
        StateScoped(act),
    ));
    // Тёмная рама: два столба и балка.
    let frame = materials.add(StandardMaterial {
        base_color: Color::srgb(0.05, 0.05, 0.07),
        perceptual_roughness: 0.9,
        ..default()
    });
    for side in [-1.0f32, 1.0] {
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(0.3, 3.2, 0.5))),
            MeshMaterial3d(frame.clone()),
            Transform::from_translation(spot + Vec3::new(1.35 * side, 1.6, 0.0)),
            StateScoped(act),
        ));
    }
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(3.0, 0.3, 0.5))),
        MeshMaterial3d(frame.clone()),
        Transform::from_translation(spot + Vec3::new(0.0, 3.2, 0.0)),
        StateScoped(act),
    ));
    // Яркий свет-маяк над порталом (видно издалека даже во тьме).
    commands.spawn((
        PointLight {
            color: Color::srgb(0.9, 0.95, 1.0),
            intensity: 150_000.0,
            range: 22.0,
            ..default()
        },
        Transform::from_translation(spot + Vec3::new(0.0, 2.2, 0.0)),
        StateScoped(act),
    ));
}

// ---------------------------------------------------------------------------
// HUD
// ---------------------------------------------------------------------------

/// Построение HUD: виньетка безумия, маленькая тусклая полоска стамины
/// и кино-оверлеи (виньетка + зерно плёнки). Никаких подсказок, прицела
/// и счётчиков - только стамина. Всё удаляется автоматически при выходе.
fn setup_hud(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    settings: Res<GameSettings>,
    state: Res<State<GameState>>,
) {
    let act = *state.get();
    // Виньетка безумия - первой, чтобы лежать ПОД остальным HUD.
    commands.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        BackgroundColor(Color::srgba(0.55, 0.02, 0.03, 0.0)),
        InsanityVignette,
        StateScoped(act),
    ));

    // Панель стамины (снизу по центру): только маленькая тусклая полоска.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(26.0),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            StateScoped(act),
        ))
        .with_children(|parent| {
            parent
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        row_gap: Val::Px(6.0),
                        ..default()
                    },
                    StateScoped(act),
                ))
                .with_children(|panel| {
                    panel
                        .spawn((
                            Node {
                                width: Val::Px(150.0),
                                height: Val::Px(5.0),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
                            StateScoped(act),
                        ))
                        .with_children(|bar| {
                            bar.spawn((
                                Node {
                                    width: Val::Percent(100.0),
                                    height: Val::Percent(100.0),
                                    ..default()
                                },
                                BackgroundColor(Color::srgb(0.25, 0.45, 0.15)),
                                StaminaFill,
                                StateScoped(act),
                            ));
                        });
                });
        });

    // Постоянная тёмная виньетка + зерно плёнки поверх остального HUD.
    // Акт 3: тонкая красная полоска таймера реактора над стаминой.
    if act == GameState::Act3_TheReactor {
        commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(42.0),
                    width: Val::Percent(100.0),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                StateScoped(act),
            ))
            .with_children(|parent| {
                parent
                    .spawn((
                        Node {
                            width: Val::Px(150.0),
                            height: Val::Px(5.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.25, 0.02, 0.02, 0.55)),
                        StateScoped(act),
                    ))
                    .with_children(|bar| {
                        bar.spawn((
                            Node {
                                width: Val::Percent(100.0),
                                height: Val::Percent(100.0),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.75, 0.08, 0.06, 0.8)),
                            MeltdownFill,
                            StateScoped(act),
                        ));
                    });
            });
    }

    atmosphere::spawn_cinematic_overlays(&mut commands, &mut images, &settings, act);
}

// ---------------------------------------------------------------------------
// Фрагменты, переходы, реактор, лифт, финалы
// ---------------------------------------------------------------------------

/// Подбор фрагментов (Акт 2). Когда собраны все - сирена и переход в Акт 3.
fn collect_fragments(
    mut commands: Commands,
    state: Res<State<GameState>>,
    players: Query<&Transform, With<Player>>,
    fragments: Query<(Entity, &Transform), With<EchoFragment>>,
    mut progress: ResMut<GameProgress>,
    transition: Option<ResMut<PendingTransition>>,
) {
    if transition.is_some() || progress.collected >= progress.total {
        return;
    }
    for player in &players {
        for (entity, transform) in &fragments {
            let dx = player.translation.x - transform.translation.x;
            let dz = player.translation.z - transform.translation.z;
            if dx * dx + dz * dz < PICKUP_RADIUS * PICKUP_RADIUS {
                commands.entity(entity).despawn();
                progress.collected += 1;
                info!(
                    "Echo fragment {}/{} collected",
                    progress.collected, progress.total
                );
            }
        }
    }
    if progress.collected >= progress.total {
        commands.insert_resource(PendingTransition {
            timer: Timer::from_seconds(2.5, TimerMode::Once),
            next: GameState::Act3_TheReactor,
        });
        interaction::spawn_subtitle(
            &mut commands,
            "THE REACTOR IS GOING CRITICAL",
            "Emergency overload started. Everything goes dark in seconds. RUN.",
            4.0,
            *state.get(),
        );
        info!("All fragments - the reactor wakes");
    }
}

// ---------------------------------------------------------------------------
// Забег: сброс, переходы между актами, реактор, лифт, рестарт
// ---------------------------------------------------------------------------

/// Сброс забега при входе в Акт 1 (каждый вход в Акт 1 - новый забег):
/// прогресс, скримеры, отложенный переход, свет окружения.
fn reset_run(
    mut commands: Commands,
    mut progress: ResMut<GameProgress>,
    mut screamer: ResMut<ScreamerState>,
    mut ambient: ResMut<AmbientLight>,
) {
    *progress = GameProgress::default();
    screamer.full_reset();
    commands.remove_resource::<PendingTransition>();
    ambient.color = Color::srgb(0.5, 0.58, 0.75);
    ambient.brightness = 0.09;
}

/// Взвод таймера реактора при входе в Акт 3.
fn reset_reactor(mut commands: Commands) {
    commands.insert_resource(ReactorTimer {
        time_left: REACTOR_SECS,
    });
}

/// Отложенные переходы между актами (рычаг, фрагменты).
fn tick_transition(
    time: Res<Time>,
    mut commands: Commands,
    transition: Option<ResMut<PendingTransition>>,
    mut next_state: ResMut<NextState<GameState>>,
) {
    let Some(mut transition) = transition else {
        return;
    };
    transition.timer.tick(time.delta());
    if transition.timer.just_finished() {
        let next = transition.next;
        commands.remove_resource::<PendingTransition>();
        next_state.set(next);
        info!("Act transition");
    }
}

/// Таймер реактора Акта 3: тикает вниз, багровеет свет, тает полоска.
/// На нуле - тепловой взрыв (финал-поражение).
fn tick_reactor(
    time: Res<Time>,
    state: Res<State<GameState>>,
    reactor: Option<ResMut<ReactorTimer>>,
    mut ambient: ResMut<AmbientLight>,
    mut progress: ResMut<GameProgress>,
    mut next_state: ResMut<NextState<GameState>>,
    mut bars: Query<&mut Node, With<MeltdownFill>>,
) {
    if *state.get() != GameState::Act3_TheReactor {
        return;
    }
    let Some(mut reactor) = reactor else {
        return;
    };
    reactor.time_left = (reactor.time_left - time.delta_secs()).max(0.0);
    // Чем ближе взрыв, тем багровее свет.
    let dread = 1.0 - reactor.time_left / REACTOR_SECS;
    ambient.color = Color::srgb(
        0.5 + 0.4 * dread,
        0.58 - 0.35 * dread,
        0.75 - 0.55 * dread,
    );
    for mut node in &mut bars {
        node.width = Val::Percent((reactor.time_left / REACTOR_SECS * 100.0).clamp(0.0, 100.0));
    }
    if reactor.time_left <= 0.0 {
        progress.ending = Some(EndingKind::Timeout);
        next_state.set(GameState::GameOver);
        info!("The reactor blew.");
    }
}

/// Триггер лифта (Акт 3): 5 реликвий - Искупление, иначе - Поглощение.
fn elevator_trigger(
    state: Res<State<GameState>>,
    players: Query<&Transform, With<Player>>,
    lifts: Query<&Transform, (With<Elevator>, Without<Player>)>,
    mut progress: ResMut<GameProgress>,
    mut next_state: ResMut<NextState<GameState>>,
) {
    if *state.get() != GameState::Act3_TheReactor {
        return;
    }
    for player in &players {
        for lift in &lifts {
            let dx = player.translation.x - lift.translation.x;
            let dz = player.translation.z - lift.translation.z;
            if dx * dx + dz * dz < ELEVATOR_RADIUS * ELEVATOR_RADIUS {
                if progress.quest_items >= interaction::QUEST_COUNT as u32 {
                    next_state.set(GameState::GameWon);
                    info!("REDEMPTION");
                } else {
                    progress.ending = Some(EndingKind::Absorbed);
                    next_state.set(GameState::GameOver);
                    info!("ABSORBED");
                }
                return;
            }
        }
    }
}

/// Рестарт забега клавишей R (в актах и на финалах): начинается Акт 1,
/// всё остальное сбрасывает `reset_run` на входе в него.
fn restart_run(keys: Res<ButtonInput<KeyCode>>, mut next_state: ResMut<NextState<GameState>>) {
    if keys.just_pressed(KeyCode::KeyR) {
        next_state.set(GameState::Act1_TheDescent);
        info!("Run restarted from Act 1");
    }
}

/// Экран финала (GameOver/GameWon): 2D-камера + затемнение + табличка.
/// Текст поражения зависит от причины (`GameProgress.ending`).
fn setup_end_screen(
    mut commands: Commands,
    progress: Res<GameProgress>,
    state: Res<State<GameState>>,
) {
    let act = *state.get();
    let won = act == GameState::GameWon;
    let (title, title_color, body): (&str, Color, &str) = if won {
        (
            "REDEMPTION",
            Color::srgb(0.85, 0.75, 0.5),
            "You hold the light steady and speak: \"I won't run anymore. Forgive me, Misha.\" The shadow melts into a small smiling boy - and is gone. The lift rises into bright dawn. You are free.",
        )
    } else {
        match progress.ending {
            Some(EndingKind::HeartDeath) => (
                "YOU DIED",
                Color::srgb(0.8, 0.05, 0.05),
                "Your heart gave out in the dark.",
            ),
            Some(EndingKind::Timeout) => (
                "THE REACTOR BLEW",
                Color::srgb(0.85, 0.3, 0.1),
                "White heat takes the bunker. You never reached the lift.",
            ),
            _ => (
                "ABSORBED",
                Color::srgb(0.45, 0.05, 0.1),
                "Black hands unfold from the dark lift. Without all five keepsakes, guilt keeps its hold - and the shadow drags you down the shaft.",
            ),
        }
    };
    commands.spawn((Camera2d, StateScoped(act)));
    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.88)),
            StateScoped(act),
        ))
        .with_children(|parent| {
            parent
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        row_gap: Val::Px(14.0),
                        margin: UiRect::horizontal(Val::Px(60.0)),
                        ..default()
                    },
                    StateScoped(act),
                ))
                .with_children(|panel| {
                    panel.spawn((
                        Text::new(title),
                        TextFont {
                            font_size: 64.0,
                            ..default()
                        },
                        TextColor(title_color),
                        StateScoped(act),
                    ));
                    panel.spawn((
                        Text::new(body),
                        TextFont {
                            font_size: 20.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.75, 0.75, 0.78)),
                        StateScoped(act),
                    ));
                    if !won {
                        panel.spawn((
                            Text::new(format!(
                                "KEEPSAKES: {}/{}",
                                progress.quest_items,
                                interaction::QUEST_COUNT
                            )),
                            TextFont {
                                font_size: 20.0,
                                ..default()
                            },
                            TextColor(Color::srgb(0.8, 0.65, 0.4)),
                            StateScoped(act),
                        ));
                    }
                    panel.spawn((
                        Text::new("R - NEW RUN      ESC - MENU"),
                        TextFont {
                            font_size: 22.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.6, 0.6, 0.62)),
                        StateScoped(act),
                    ));
                });
        });
}

/// Выход в меню по Esc (из актов и с финалов).
fn escape_to_menu(
    keys: Res<ButtonInput<KeyCode>>,
    mut next_state: ResMut<NextState<GameState>>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        next_state.set(GameState::MainMenu);
    }
}

/// Лёгкое парение фрагментов (чисто визуальное).
fn bob_fragments(time: Res<Time>, mut query: Query<&mut Transform, With<EchoFragment>>) {
    for mut transform in &mut query {
        transform.translation.y =
            1.2 + (time.elapsed_secs() * 2.0 + transform.translation.x).sin() * 0.15;
    }
}

// ---------------------------------------------------------------------------
// Курсор
// ---------------------------------------------------------------------------

/// В игре: курсор захвачен и скрыт (обзор мышью).
fn grab_cursor(mut windows: Query<&mut Window>) {
    for mut window in &mut windows {
        window.cursor_options.grab_mode = CursorGrabMode::Locked;
        window.cursor_options.visible = false;
    }
}

/// Вне игры: курсор видим и свободен.
fn release_cursor(mut windows: Query<&mut Window>) {
    for mut window in &mut windows {
        window.cursor_options.grab_mode = CursorGrabMode::None;
        window.cursor_options.visible = true;
    }
}
