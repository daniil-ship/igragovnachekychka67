//! «Эхо Забытых Стен» — психологический хоррор от первого лица.
//!
//! Точка входа: окно, глобальные состояния [`AppState`], подключение плагинов,
//! главное меню, процедурный уровень-лабиринт, HUD и экран победы.
//!
//! Архитектура (ECS, по модулям):
//! - [`player`] — игрок: движение, стамина, принудительный бег, фонарик, HUD;
//! - [`audio`] — звукорежиссёр: музыка меню и циклический амбиент;
//! - [`hallucinations`] — безумие и скримеры (PNG на весь экран + MP3).

mod audio;
mod hallucinations;
mod player;

use audio::AudioDirectorPlugin;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, WindowPlugin, WindowResolution};
use hallucinations::{HallucinationsPlugin, Insanity, ScreamerState};
use player::{
    Flashlight, ForcedRun, FragmentCounterText, InsanityVignette, Player, PlayerPlugin, Stamina,
    StaminaFill, WarningText,
};
use rand::seq::SliceRandom;
use rand::Rng;

// ---------------------------------------------------------------------------
// Константы мира
// ---------------------------------------------------------------------------

/// Размер одной клетки лабиринта в метрах.
pub const CELL: f32 = 4.0;
/// Высота стен лабиринта.
pub const WALL_H: f32 = 3.2;
/// Размер лабиринта в клетках (только НЕЧЁТНЫЕ числа — требование генератора).
const MAZE_W: usize = 21;
const MAZE_H: usize = 15;
/// Сколько фрагментов эха нужно собрать для победы.
pub const FRAGMENT_COUNT: usize = 5;
/// Дистанция сбора фрагмента (по горизонтали), метры.
const PICKUP_RADIUS: f32 = 1.5;

// ---------------------------------------------------------------------------
// Состояния, ресурсы, компоненты уровня
// ---------------------------------------------------------------------------

/// Глобальные состояния игры.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum AppState {
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
    /// Точки спавна фрагментов (для рестарта клавишей R).
    pub fragment_spots: Vec<Vec3>,
}

impl Default for GameProgress {
    fn default() -> Self {
        Self {
            collected: 0,
            total: FRAGMENT_COUNT as u32,
            won: false,
            fragment_spots: Vec::new(),
        }
    }
}

/// Светящийся фрагмент эха — цель забега (5 штук в дальних углах лабиринта).
#[derive(Component)]
struct EchoFragment;

/// Маркер оверлея победы (для удаления при рестарте).
#[derive(Component)]
struct WinOverlay;

/// Маркер мигающей подсказки в меню.
#[derive(Component)]
struct MenuPrompt;

// ---------------------------------------------------------------------------
// Точка входа
// ---------------------------------------------------------------------------

fn main() {
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
        .init_state::<AppState>()
        .add_plugins((PlayerPlugin, AudioDirectorPlugin, HallucinationsPlugin))
        // Холодный тусклый свет окружения.
        .add_systems(Startup, setup_ambient_light)
        // Главное меню.
        .add_systems(OnEnter(AppState::MainMenu), setup_menu)
        .add_systems(
            Update,
            (menu_input, blink_menu_prompt).run_if(in_state(AppState::MainMenu)),
        )
        // Игра: генерация, HUD, курсор, сбор фрагментов, победа, выход.
        .add_systems(OnEnter(AppState::InGame), (generate_level, setup_hud, grab_cursor))
        .add_systems(
            Update,
            (collect_fragments, win_input, escape_to_menu, bob_fragments)
                .run_if(in_state(AppState::InGame)),
        )
        .add_systems(OnExit(AppState::InGame), release_cursor)
        .run();
}

/// Run-условие «управление игрока разрешено»: мы в игре, скример не активен
/// (его секунда блокирует ввод) и забег ещё не выигран.
pub fn game_input_allowed(
    state: Res<State<AppState>>,
    screamer: Res<ScreamerState>,
    progress: Res<GameProgress>,
) -> bool {
    *state.get() == AppState::InGame && !screamer.active && !progress.won
}

// ---------------------------------------------------------------------------
// Свет окружения
// ---------------------------------------------------------------------------

/// Едва заметный холодный свет, чтобы тьма не была абсолютно чёрной.
/// (Ресурс уже создан `PbrPlugin`, мы лишь приглушаем его.)
fn setup_ambient_light(mut ambient_light: ResMut<AmbientLight>) {
    ambient_light.color = Color::srgb(0.7, 0.75, 1.0);
    ambient_light.brightness = 0.04;
}

// ---------------------------------------------------------------------------
// Главное меню
// ---------------------------------------------------------------------------

/// Построение меню: 2D-камера + заголовок, подсказка и управление.
fn setup_menu(mut commands: Commands, mut windows: Query<&mut Window>) {
    // В меню курсор всегда видим и свободен.
    for mut window in &mut windows {
        window.cursor_options.grab_mode = CursorGrabMode::None;
        window.cursor_options.visible = true;
    }

    commands.spawn((Camera2d, DespawnOnExit(AppState::MainMenu)));

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
            DespawnOnExit(AppState::MainMenu),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("ECHO OF FORGOTTEN WALLS"),
                TextFont {
                    font_size: 60.0,
                    ..default()
                },
                TextColor(Color::srgb(0.72, 0.08, 0.08)),
                DespawnOnExit(AppState::MainMenu),
            ));
            parent.spawn((
                Text::new("A PSYCHOLOGICAL HORROR"),
                TextFont {
                    font_size: 24.0,
                    ..default()
                },
                TextColor(Color::srgb(0.45, 0.42, 0.45)),
                DespawnOnExit(AppState::MainMenu),
            ));
            parent.spawn((
                Text::new("PRESS ENTER OR CLICK TO BEGIN"),
                TextFont {
                    font_size: 28.0,
                    ..default()
                },
                TextColor(Color::srgb(0.85, 0.8, 0.78)),
                MenuPrompt,
                DespawnOnExit(AppState::MainMenu),
            ));
            parent.spawn((
                Text::new(
                    "WASD / ARROWS — MOVE      MOUSE — LOOK\n\
                     SHIFT — RUN (DRAINS STAMINA)\n\
                     BEWARE: RUNNING ON EMPTY STAMINA FEEDS YOUR MADNESS\n\
                     FIND 5 ECHO FRAGMENTS TO ESCAPE      ESC — MENU",
                ),
                TextFont {
                    font_size: 18.0,
                    ..default()
                },
                TextColor(Color::srgb(0.5, 0.48, 0.5)),
                DespawnOnExit(AppState::MainMenu),
            ));
            parent.spawn((
                Text::new("HEADPHONES RECOMMENDED"),
                TextFont {
                    font_size: 16.0,
                    ..default()
                },
                TextColor(Color::srgb(0.5, 0.12, 0.12)),
                DespawnOnExit(AppState::MainMenu),
            ));
        });
}

/// Вход в игру по Enter/Space/клику.
fn menu_input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    if keys.just_pressed(KeyCode::Enter)
        || keys.just_pressed(KeyCode::Space)
        || mouse.just_pressed(MouseButton::Left)
    {
        next_state.set(AppState::InGame);
    }
}

/// Мигание подсказки «PRESS ENTER...» (без таймера — по времени кадра).
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

// ---------------------------------------------------------------------------
// Генерация уровня
// ---------------------------------------------------------------------------

/// Центр клетки лабиринта в мировых координатах (x, z). Карта центрирована
/// в начале координат.
fn cell_center(cx: usize, cy: usize) -> (f32, f32) {
    (
        (cx as f32 - MAZE_W as f32 / 2.0 + 0.5) * CELL,
        (cy as f32 - MAZE_H as f32 / 2.0 + 0.5) * CELL,
    )
}

/// Генератор лабиринта: итеративный backtracker (`true` — стена).
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

    // Заплетаем ~35% тупиков: сносим одну стену — появляются петли.
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

/// Построение уровня при входе в игру: пол, потолок, стены, игрок с
/// фонариком и 5 фрагментов эха в дальних клетках. Каждый забег — новый
/// случайный лабиринт.
fn generate_level(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut colliders: ResMut<LevelColliders>,
    mut progress: ResMut<GameProgress>,
) {
    // Сброс состояния забега (важно при повторном входе из меню).
    colliders.walls.clear();
    progress.collected = 0;
    progress.won = false;
    progress.total = FRAGMENT_COUNT as u32;
    progress.fragment_spots.clear();

    let maze = generate_maze(MAZE_W, MAZE_H);

    // Общие материалы (создаём один раз на уровень).
    let floor_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.09, 0.09, 0.11),
        perceptual_roughness: 0.95,
        ..default()
    });
    let wall_mat_a = materials.add(StandardMaterial {
        base_color: Color::srgb(0.16, 0.15, 0.17),
        perceptual_roughness: 0.9,
        ..default()
    });
    let wall_mat_b = materials.add(StandardMaterial {
        base_color: Color::srgb(0.12, 0.11, 0.13),
        perceptual_roughness: 0.9,
        ..default()
    });

    let world_w = MAZE_W as f32 * CELL;
    let world_d = MAZE_H as f32 * CELL;

    // Пол и потолок — две плиты на всю карту.
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(world_w, 0.2, world_d))),
        MeshMaterial3d(floor_mat.clone()),
        Transform::from_xyz(0.0, -0.1, 0.0),
        DespawnOnExit(AppState::InGame),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(world_w, 0.2, world_d))),
        MeshMaterial3d(floor_mat),
        Transform::from_xyz(0.0, WALL_H + 0.1, 0.0),
        DespawnOnExit(AppState::InGame),
    ));

    // Стены: по кубу на каждую клетку-стену + коллайдер.
    for (cy, row) in maze.iter().enumerate() {
        for (cx, &is_wall) in row.iter().enumerate() {
            if !is_wall {
                continue;
            }
            let (x, z) = cell_center(cx, cy);
            let mat = if (cx + cy) % 2 == 0 {
                wall_mat_a.clone()
            } else {
                wall_mat_b.clone()
            };
            commands.spawn((
                Mesh3d(meshes.add(Cuboid::new(CELL, WALL_H, CELL))),
                MeshMaterial3d(mat),
                Transform::from_xyz(x, WALL_H / 2.0, z),
                DespawnOnExit(AppState::InGame),
            ));
            colliders.walls.push(WallAabb {
                min_x: x - CELL / 2.0,
                max_x: x + CELL / 2.0,
                min_z: z - CELL / 2.0,
                max_z: z + CELL / 2.0,
            });
        }
    }

    // Игрок в клетке (1, 1): смотрим в открытый проход —
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
            Transform {
                translation: Vec3::new(spawn_x, player::EYE_HEIGHT, spawn_z),
                rotation: Quat::from_euler(EulerRot::YXZ, yaw, 0.0, 0.0),
                ..default()
            },
            Player { yaw, pitch: 0.0 },
            Stamina::default(),
            ForcedRun(false),
            Insanity(0.0),
            DespawnOnExit(AppState::InGame),
        ))
        .with_children(|parent| {
            // Фонарик — спотлайт, ребёнок камеры (светит туда же, куда смотрим).
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
                DespawnOnExit(AppState::InGame),
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
        // Лабиринт тесный — fallback: просто самые дальние клетки пола.
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
            intensity: 30.0,
            range: 7.0,
            ..default()
        },
        EchoFragment,
        DespawnOnExit(AppState::InGame),
    ));
}

// ---------------------------------------------------------------------------
// HUD
// ---------------------------------------------------------------------------

/// Построение HUD: виньетка безумия, прицел, счётчик, подсказка и панель
/// стамины. Все элементы удаляются автоматически при выходе из игры.
fn setup_hud(mut commands: Commands, progress: Res<GameProgress>) {
    // Виньетка безумия — первой, чтобы лежать ПОД остальным HUD.
    commands.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        BackgroundColor(Color::srgba(0.55, 0.02, 0.03, 0.0)),
        InsanityVignette,
        DespawnOnExit(AppState::InGame),
    ));

    // Прицел-точка по центру экрана.
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(50.0),
            top: Val::Percent(50.0),
            width: Val::Px(5.0),
            height: Val::Px(5.0),
            ..default()
        },
        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.7)),
        DespawnOnExit(AppState::InGame),
    ));

    // Счётчик фрагментов (сверху слева).
    let counter_text = format!("FRAGMENTS: {}/{}", progress.collected, progress.total);
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(18.0),
                top: Val::Px(16.0),
                ..default()
            },
            DespawnOnExit(AppState::InGame),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new(counter_text),
                TextFont {
                    font_size: 26.0,
                    ..default()
                },
                TextColor(Color::srgb(0.75, 0.85, 0.9)),
                FragmentCounterText,
                DespawnOnExit(AppState::InGame),
            ));
        });

    // Подсказка управления (снизу слева).
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(18.0),
                bottom: Val::Px(16.0),
                ..default()
            },
            DespawnOnExit(AppState::InGame),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("WASD — MOVE   SHIFT — RUN   MOUSE — LOOK   ESC — MENU"),
                TextFont {
                    font_size: 15.0,
                    ..default()
                },
                TextColor(Color::srgba(1.0, 1.0, 1.0, 0.45)),
                DespawnOnExit(AppState::InGame),
            ));
        });

    // Панель стамины (снизу по центру): предупреждение + полоска + подпись.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(26.0),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            DespawnOnExit(AppState::InGame),
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
                    DespawnOnExit(AppState::InGame),
                ))
                .with_children(|panel| {
                    panel.spawn((
                        Text::new(""),
                        TextFont {
                            font_size: 22.0,
                            ..default()
                        },
                        TextColor(Color::srgb(1.0, 0.25, 0.2)),
                        WarningText,
                        DespawnOnExit(AppState::InGame),
                    ));
                    panel
                        .spawn((
                            Node {
                                width: Val::Px(340.0),
                                height: Val::Px(18.0),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.65)),
                            DespawnOnExit(AppState::InGame),
                        ))
                        .with_children(|bar| {
                            bar.spawn((
                                Node {
                                    width: Val::Percent(100.0),
                                    height: Val::Percent(100.0),
                                    ..default()
                                },
                                BackgroundColor(Color::srgb(0.2, 0.8, 0.25)),
                                StaminaFill,
                                DespawnOnExit(AppState::InGame),
                            ));
                        });
                    panel.spawn((
                        Text::new("STAMINA"),
                        TextFont {
                            font_size: 14.0,
                            ..default()
                        },
                        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.55)),
                        DespawnOnExit(AppState::InGame),
                    ));
                });
        });
}

// ---------------------------------------------------------------------------
// Фрагменты, победа, выход
// ---------------------------------------------------------------------------

/// Подбор фрагментов при приближении. Когда собраны все — победа.
fn collect_fragments(
    mut commands: Commands,
    players: Query<&Transform, With<Player>>,
    fragments: Query<(Entity, &Transform), With<EchoFragment>>,
    mut progress: ResMut<GameProgress>,
) {
    if progress.won {
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
            DespawnOnExit(AppState::InGame),
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
                    DespawnOnExit(AppState::InGame),
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
                        DespawnOnExit(AppState::InGame),
                    ));
                    panel.spawn((
                        Text::new("R — PLAY AGAIN      ESC — MENU"),
                        TextFont {
                            font_size: 24.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.55, 0.55, 0.6)),
                        WinOverlay,
                        DespawnOnExit(AppState::InGame),
                    ));
                });
        });
}

/// Сброс состояния скриммеров при входе в игру. Защита от «залипания»
/// флага активности, если игрок вышел в меню прямо во время скримера.
fn reset_screamer_state(mut screamer: ResMut<ScreamerState>) {
    screamer.full_reset();
}

/// Рестарт забега клавишей R после победы: убираем оверлей, возвращаем
/// фрагменты на места, сбрасываем стамину/безумие/скримеры.
fn win_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    overlays: Query<Entity, With<WinOverlay>>,
    screamer_overlays: Query<Entity, With<ScreamerOverlay>>,
    mut progress: ResMut<GameProgress>,
    mut screamer: ResMut<ScreamerState>,
    mut stamina_query: Query<&mut Stamina>,
    mut insanity_query: Query<&mut Insanity>,
    mut forced_query: Query<&mut ForcedRun>,
) {
    if !progress.won || !keys.just_pressed(KeyCode::KeyR) {
        return;
    }
    for entity in &overlays {
        commands.entity(entity).despawn();
    }
    // Победа могла случиться прямо во время скримера — убираем и его картинку,
    // иначе она зависнет (тиканье ниже будет сброшено).
    for entity in &screamer_overlays {
        commands.entity(entity).despawn();
    }
    progress.collected = 0;
    progress.won = false;
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
