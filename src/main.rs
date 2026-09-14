//! «Эхо Забытых Стен» - психологический хоррор от первого лица.
//!
//! Точка входа: окно, глобальные состояния [`AppState`], подключение плагинов,
//! главное меню, процедурный уровень-лабиринт, HUD и экран победы.
//!
//! Архитектура (ECS, по модулям):
//! - [`player`] - игрок: движение, стамина, принудительный бег, фонарик, HUD;
//! - [`audio`] - звукорежиссёр: музыка меню и циклический амбиент;
//! - [`hallucinations`] - безумие и скримеры (PNG на весь экран + MP3).
//! - [`atmosphere`] - лампы, туман, пыль, виньетка и зерно плёнки.

mod atmosphere;
mod audio;
mod hallucinations;
mod player;
mod shadow;
mod textures;

use std::path::PathBuf;

use atmosphere::AtmospherePlugin;
use audio::AudioDirectorPlugin;
use bevy::prelude::*;
use bevy::state::state_scoped::StateScoped;
use bevy::window::{CursorGrabMode, WindowPlugin, WindowResolution};
use hallucinations::{HallucinationsPlugin, Insanity, ScreamerOverlay, ScreamerState};
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
/// Сколько фрагментов эха нужно собрать для победы.
pub const FRAGMENT_COUNT: usize = 5;
/// Дистанция сбора фрагмента (по горизонтали), метры.
const PICKUP_RADIUS: f32 = 1.5;
/// PNG-текстуры заброшки (пути внутри `assets/`). Если файла нет -
/// используется процедурная текстура из [`textures`].
pub const TEX_WALL_A: &str = "textures/wall1.png";
pub const TEX_WALL_B: &str = "textures/wall2.png";
pub const TEX_FLOOR: &str = "textures/floor.png";
pub const TEX_CEIL: &str = "textures/ceil.png";

// ---------------------------------------------------------------------------
// Состояния, ресурсы, компоненты уровня
// ---------------------------------------------------------------------------

/// Глобальные состояния игры.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(crate) enum AppState {
    /// Главное меню (музыка `menu.mp3`).
    #[default]
    MainMenu,
    /// Игра (циклический амбиент, лабиринт, скримеры).
    InGame,
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

/// Прогресс забега: собранные фрагменты и флаг победы.
#[derive(Resource)]
pub struct GameProgress {
    pub collected: u32,
    pub total: u32,
    pub won: bool,
    /// Герой мёртв (25% в момент истощения стамины).
    pub dead: bool,
    /// Точки спавна фрагментов (для рестарта клавишей R).
    pub fragment_spots: Vec<Vec3>,
}

impl Default for GameProgress {
    fn default() -> Self {
        Self {
            collected: 0,
            total: FRAGMENT_COUNT as u32,
            won: false,
            dead: false,
            fragment_spots: Vec::new(),
        }
    }
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

/// Светящийся фрагмент эха - цель забега (5 штук в дальних углах лабиринта).
#[derive(Component)]
struct EchoFragment;

/// Маркер оверлея победы (для удаления при рестарте).
#[derive(Component)]
struct WinOverlay;

/// Маркер оверлея смерти (для удаления при рестарте).
#[derive(Component)]
struct DeathOverlay;

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
        .init_state::<AppState>()
        // В Bevy 0.16 state-scoped сущности включаются явно,
        // иначе маркеры StateScoped не будут ничего удалять.
        .enable_state_scoped_entities::<AppState>()
        .add_plugins((
            PlayerPlugin,
            AudioDirectorPlugin,
            AtmospherePlugin,
            HallucinationsPlugin,
            ShadowPlugin,
        ))
        // Холодный тусклый свет окружения.
        .add_systems(Startup, setup_ambient_light)
        // Главное меню.
        .add_systems(OnEnter(AppState::MainMenu), setup_menu)
        .add_systems(
            Update,
            (menu_input, blink_menu_prompt, settings_buttons).run_if(in_state(AppState::MainMenu)),
        )
        // Игра: генерация, HUD, курсор, сбор фрагментов, победа, выход.
        .add_systems(OnEnter(AppState::InGame), (generate_level, setup_hud, grab_cursor))
        .add_systems(
            Update,
            (collect_fragments, restart_input, escape_to_menu, bob_fragments)
                .run_if(in_state(AppState::InGame)),
        )
        .add_systems(OnExit(AppState::InGame), release_cursor)
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

/// Run-условие «управление игрока разрешено»: мы в игре, скример не активен
/// (его секунда блокирует ввод) и забег ещё не выигран.
pub fn game_input_allowed(
    state: Res<State<AppState>>,
    screamer: Res<ScreamerState>,
    progress: Res<GameProgress>,
) -> bool {
    *state.get() == AppState::InGame && !screamer.active && !progress.won && !progress.dead
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

    commands.spawn((Camera2d, StateScoped(AppState::MainMenu)));

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
            StateScoped(AppState::MainMenu),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("ECHO OF FORGOTTEN WALLS"),
                TextFont {
                    font_size: 60.0,
                    ..default()
                },
                TextColor(Color::srgb(0.72, 0.08, 0.08)),
                StateScoped(AppState::MainMenu),
            ));
            parent.spawn((
                Text::new("A PSYCHOLOGICAL HORROR"),
                TextFont {
                    font_size: 24.0,
                    ..default()
                },
                TextColor(Color::srgb(0.45, 0.42, 0.45)),
                StateScoped(AppState::MainMenu),
            ));
            parent.spawn((
                Text::new("PRESS ENTER OR CLICK TO BEGIN"),
                TextFont {
                    font_size: 28.0,
                    ..default()
                },
                TextColor(Color::srgb(0.85, 0.8, 0.78)),
                MenuPrompt,
                StateScoped(AppState::MainMenu),
            ));
            parent.spawn((
                Text::new(
                    "WASD / ARROWS - MOVE      MOUSE - LOOK\n\
                     SHIFT - RUN (DRAINS STAMINA)\n\
                     BEWARE: RUNNING ON EMPTY STAMINA FEEDS YOUR MADNESS\n\
                     FIND 5 ECHO FRAGMENTS TO ESCAPE      ESC - MENU",
                ),
                TextFont {
                    font_size: 18.0,
                    ..default()
                },
                TextColor(Color::srgb(0.5, 0.48, 0.5)),
                StateScoped(AppState::MainMenu),
            ));
            parent.spawn((
                Text::new("HEADPHONES RECOMMENDED"),
                TextFont {
                    font_size: 16.0,
                    ..default()
                },
                TextColor(Color::srgb(0.5, 0.12, 0.12)),
                StateScoped(AppState::MainMenu),
            ));
            // Настройки экрана: кнопки-переключатели (клик по ним не стартует игру).
            parent.spawn((
                Text::new("- SETTINGS -"),
                TextFont {
                    font_size: 20.0,
                    ..default()
                },
                TextColor(Color::srgb(0.6, 0.55, 0.55)),
                StateScoped(AppState::MainMenu),
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
                        StateScoped(AppState::MainMenu),
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
                            StateScoped(AppState::MainMenu),
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
    mut next_state: ResMut<NextState<AppState>>,
) {
    if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space) {
        next_state.set(AppState::InGame);
        return;
    }
    if mouse.just_pressed(MouseButton::Left) {
        let on_button = buttons
            .iter()
            .any(|i| *i == Interaction::Hovered || *i == Interaction::Pressed);
        if !on_button {
            next_state.set(AppState::InGame);
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

/// Построение уровня при входе в игру: пол, потолок, стены, игрок с
/// фонариком и 5 фрагментов эха в дальних клетках. Каждый забег - новый
/// случайный лабиринт.
fn generate_level(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    assets: Res<AssetServer>,
    mut colliders: ResMut<LevelColliders>,
    mut progress: ResMut<GameProgress>,
) {
    // Сброс состояния забега (важно при повторном входе из меню).
    colliders.walls.clear();
    progress.collected = 0;
    progress.won = false;
    progress.dead = false;
    progress.total = FRAGMENT_COUNT as u32;
    progress.fragment_spots.clear();

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
                    StateScoped(AppState::InGame),
                ));
                commands.spawn((
                    Mesh3d(meshes.add(Cuboid::new(CELL, 0.2, CELL))),
                    MeshMaterial3d(ceil_mat.clone()),
                    Transform::from_xyz(x, WALL_H + 0.1, z),
                    StateScoped(AppState::InGame),
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
                StateScoped(AppState::InGame),
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
            StateScoped(AppState::InGame),
        ))
        .with_children(|parent| {
            // Фонарик - спотлайт, ребёнок камеры (светит туда же, куда смотрим).
            parent.spawn((
                SpotLight {
                    color: Color::srgb(1.0, 0.95, 0.85),
                    intensity: player::FLASHLIGHT_INTENSITY,
                    range: 32.0,
                    inner_angle: 0.22,
                    outer_angle: 0.55,
                    shadows_enabled: true,
                    ..default()
                },
                Transform::from_xyz(0.15, -0.2, 0.0),
                Flashlight {
                    base_intensity: player::FLASHLIGHT_INTENSITY,
                },
                StateScoped(AppState::InGame),
            ));
        });

    // Фрагменты эха: случайные клетки пола вдалеке от спавна.
    let mut rng = rand::thread_rng();
    let mut candidates: Vec<(usize, usize)> = Vec::new();
    for (cy, row) in maze.iter().enumerate() {
        for (cx, &is_wall) in row.iter().enumerate() {
            if is_wall {
                continue;
            }
            if cx.abs_diff(1) + cy.abs_diff(1) >= 14 {
                candidates.push((cx, cy));
            }
        }
    }
    if candidates.len() < FRAGMENT_COUNT {
        // Лабиринт тесный - fallback: просто самые дальние клетки пола.
        let mut all: Vec<(usize, usize)> = Vec::new();
        for (cy, row) in maze.iter().enumerate() {
            for (cx, &is_wall) in row.iter().enumerate() {
                if !is_wall {
                    all.push((cx, cy));
                }
            }
        }
        all.sort_by_key(|&(cx, cy)| {
            std::cmp::Reverse(cx.abs_diff(1) + cy.abs_diff(1))
        });
        candidates = all.into_iter().take(FRAGMENT_COUNT).collect();
    }
    candidates.shuffle(&mut rng);
    for &(cx, cy) in candidates.iter().take(FRAGMENT_COUNT) {
        let (x, z) = cell_center(cx, cy);
        let spot = Vec3::new(x, 0.0, z);
        progress.fragment_spots.push(spot);
        spawn_fragment(&mut commands, &mut meshes, &mut materials, spot);
    }

    info!(
        "Level generated: {} walls, {} fragments",
        colliders.walls.len(),
        progress.fragment_spots.len()
    );
}

/// Спавн одного фрагмента эха: светящаяся сфера + точечный свет.
fn spawn_fragment(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    spot: Vec3,
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
        StateScoped(AppState::InGame),
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
) {
    // Виньетка безумия - первой, чтобы лежать ПОД остальным HUD.
    commands.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        BackgroundColor(Color::srgba(0.55, 0.02, 0.03, 0.0)),
        InsanityVignette,
        StateScoped(AppState::InGame),
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
            StateScoped(AppState::InGame),
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
                    StateScoped(AppState::InGame),
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
                            StateScoped(AppState::InGame),
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
                                StateScoped(AppState::InGame),
                            ));
                        });
                });
        });

    // Постоянная тёмная виньетка + зерно плёнки поверх остального HUD.
    atmosphere::spawn_cinematic_overlays(&mut commands, &mut images, &settings);
}

// ---------------------------------------------------------------------------
// Фрагменты, победа, выход
// ---------------------------------------------------------------------------

/// Подбор фрагментов при приближении. Когда собраны все - победа.
fn collect_fragments(
    mut commands: Commands,
    players: Query<&Transform, With<Player>>,
    fragments: Query<(Entity, &Transform), With<EchoFragment>>,
    mut progress: ResMut<GameProgress>,
) {
    if progress.won || progress.dead {
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
        progress.won = true;
        spawn_win_overlay(&mut commands);
        info!("YOU ESCAPED THE ECHO");
    }
}

/// Оверлей победы: затемнение + табличка.
/// Каждый элемент помечен отдельно, чтобы удаление было надёжным.
fn spawn_win_overlay(commands: &mut Commands) {
    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.78)),
            WinOverlay,
            StateScoped(AppState::InGame),
        ))
        .with_children(|parent| {
            parent
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        row_gap: Val::Px(14.0),
                        padding: UiRect {
                            left: Val::Px(56.0),
                            right: Val::Px(56.0),
                            top: Val::Px(40.0),
                            bottom: Val::Px(40.0),
                        },
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.03, 0.02, 0.025)),
                    WinOverlay,
                    StateScoped(AppState::InGame),
                ))
                .with_children(|panel| {
                    panel.spawn((
                        Text::new("YOU ESCAPED THE ECHO"),
                        TextFont {
                            font_size: 46.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.8, 0.85, 0.9)),
                        WinOverlay,
                        StateScoped(AppState::InGame),
                    ));
                    panel.spawn((
                        Text::new("R - PLAY AGAIN      ESC - MENU"),
                        TextFont {
                            font_size: 24.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.55, 0.55, 0.6)),
                        WinOverlay,
                        StateScoped(AppState::InGame),
                    ));
                });
        });
}

/// Сброс состояния скриммеров при входе в игру. Защита от «залипания»
/// флага активности, если игрок вышел в меню прямо во время скримера.
fn reset_screamer_state(mut screamer: ResMut<ScreamerState>) {
    screamer.full_reset();
}

/// Оверлей смерти: красное затемнение + табличка "YOU DIED".
/// Вызывается из системы стамины в момент смертельного истощения.
fn spawn_death_overlay(commands: &mut Commands) {
    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.25, 0.0, 0.0, 0.55)),
            DeathOverlay,
            StateScoped(AppState::InGame),
        ))
        .with_children(|parent| {
            parent
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        row_gap: Val::Px(14.0),
                        padding: UiRect {
                            left: Val::Px(56.0),
                            right: Val::Px(56.0),
                            top: Val::Px(40.0),
                            bottom: Val::Px(40.0),
                        },
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.05, 0.01, 0.01)),
                    DeathOverlay,
                    StateScoped(AppState::InGame),
                ))
                .with_children(|panel| {
                    panel.spawn((
                        Text::new("YOU DIED"),
                        TextFont {
                            font_size: 64.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.8, 0.05, 0.05)),
                        DeathOverlay,
                        StateScoped(AppState::InGame),
                    ));
                    panel.spawn((
                        Text::new("YOUR HEART GAVE OUT IN THE DARK"),
                        TextFont {
                            font_size: 22.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.6, 0.5, 0.5)),
                        DeathOverlay,
                        StateScoped(AppState::InGame),
                    ));
                    panel.spawn((
                        Text::new("R - TRY AGAIN      ESC - MENU"),
                        TextFont {
                            font_size: 24.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.55, 0.55, 0.6)),
                        DeathOverlay,
                        StateScoped(AppState::InGame),
                    ));
                });
        });
}

/// Рестарт забега клавишей R после победы или смерти: убираем оверлеи,
/// возвращаем фрагменты на места, сбрасываем стамину/безумие/скримеры.
fn restart_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    overlays: Query<Entity, With<WinOverlay>>,
    screamer_overlays: Query<Entity, With<ScreamerOverlay>>,
    death_overlays: Query<Entity, With<DeathOverlay>>,
    mut progress: ResMut<GameProgress>,
    mut screamer: ResMut<ScreamerState>,
    mut stamina_query: Query<&mut Stamina>,
    mut insanity_query: Query<&mut Insanity>,
    mut forced_query: Query<&mut ForcedRun>,
) {
    if !(progress.won || progress.dead) || !keys.just_pressed(KeyCode::KeyR) {
        return;
    }
    for entity in &overlays {
        commands.entity(entity).despawn();
    }
    // Победа могла случиться прямо во время скримера - убираем и его картинку,
    // иначе она зависнет (тиканье ниже будет сброшено).
    for entity in &screamer_overlays {
        commands.entity(entity).despawn();
    }
    for entity in &death_overlays {
        commands.entity(entity).despawn();
    }
    progress.collected = 0;
    progress.won = false;
    progress.dead = false;
    for spot in progress.fragment_spots.clone() {
        spawn_fragment(&mut commands, &mut meshes, &mut materials, spot);
    }
    for mut stamina in &mut stamina_query {
        stamina.current = Stamina::MAX;
    }
    for mut insanity in &mut insanity_query {
        insanity.0 = 0.0;
    }
    for mut forced in &mut forced_query {
        forced.0 = false;
    }
    screamer.full_reset();
    info!("Run restarted");
}

/// Выход в меню по Esc из игры.
fn escape_to_menu(
    keys: Res<ButtonInput<KeyCode>>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        next_state.set(AppState::MainMenu);
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
