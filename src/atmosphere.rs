//! Потолочные лампы, тёмные зоны, туман, пыль и киноплёночные оверлеи.
//!
//! Модуль отвечает за «реалистичность» картинки:
//! - [`spawn_lamps`] расставляет по лабиринту светильники: рабочие, мигающие
//!   и разбитые, а также выбирает тёмные зоны, где света нет вообще;
//! - [`spawn_cinematic_overlays`] добавляет постоянную виньетку и зерно плёнки;
//! - системы мерцания ламп, billboard-гало, дрейфа пыли и смены кадров зерна
//!   оживляют всё это каждый кадр.

use bevy::prelude::*;
use bevy::state::state_scoped::StateScoped;
use rand::Rng;

use crate::player::Player;
use crate::{AppState, WALL_H};

// ---------------------------------------------------------------------------
// Константы баланса
// ---------------------------------------------------------------------------

/// Шаг сетки ламп в клетках: светильники ставятся в клетках
/// (1, 1), (1, 4), (4, 1), ... Все такие клетки - гарантированные проходы.
const LAMP_STEP: usize = 3;
/// Яркость потолочной лампы, канделы (пол под лампой - в ~2.6 м).
const LAMP_INTENSITY: f32 = 40_000.0;
/// Дальность света лампы, метры.
const LAMP_RANGE: f32 = 11.0;
/// Тёплый цвет света (старые лампы накаливания).
const LAMP_LIGHT_RGB: (f32, f32, f32) = (1.0, 0.74, 0.52);
/// Цвет включённой колбы (unlit, яркий).
const LAMP_BULB_RGB: (f32, f32, f32) = (1.0, 0.84, 0.62);
/// Цвет гало вокруг лампы и его базовая прозрачность.
const LAMP_GLOW_RGB: (f32, f32, f32) = (1.0, 0.72, 0.45);
const LAMP_GLOW_ALPHA: f32 = 0.5;
/// Размер гало-спрайта, метры.
const LAMP_GLOW_SIZE: f32 = 1.6;
/// Шанс, что лампа разбита: тёмная, висит криво, света нет.
const BROKEN_CHANCE: f64 = 0.15;
/// Шанс, что рабочая лампа - неисправная (мигает).
const FLICKER_CHANCE: f64 = 0.30;
/// Сколько тёмных зон (прямоугольников вообще без ламп) на уровень.
const DARK_ZONE_COUNT: usize = 2;
/// Шанс, что в тёмной зоне всё же висит разбитый плафон (для вида).
const DARK_ZONE_FIXTURE_CHANCE: f64 = 0.55;
/// Пылинок возле каждой рабочей лампы.
const DUST_PER_LAMP: usize = 6;
/// Кадров зерна плёнки и длительность каждого.
const GRAIN_FRAMES: usize = 4;
const GRAIN_FRAME_SECS: f32 = 0.09;
/// Плотность экспоненциального тумана на камере игрока.
pub(crate) const FOG_DENSITY: f32 = 0.045;

// --- Геометрия светильника (низ потолка - на высоте WALL_H) ---

/// Длина шнура от потолка до плафона.
const CORD_LEN: f32 = 0.5;
/// Высота центра шнура / плафона / колбы / источника света.
const CORD_Y: f32 = WALL_H - CORD_LEN / 2.0;
const SHADE_Y: f32 = WALL_H - CORD_LEN - 0.08;
const BULB_Y: f32 = SHADE_Y - 0.12;
const LIGHT_Y: f32 = BULB_Y - 0.05;

// ---------------------------------------------------------------------------
// Компоненты и ресурсы
// ---------------------------------------------------------------------------

/// Корень мигающей лампы: базовые цвета/яркость и состояние мерцания.
#[derive(Component)]
struct FlickerLamp {
    base_intensity: f32,
    bulb_rgb: Vec3,
    glow_rgb: Vec3,
    glow_alpha: f32,
    /// Текущий уровень света 0..1 и цель, к которой он «щёлкает».
    level: f32,
    target: f32,
    /// Время до следующего случайного переключения цели.
    retarget: f32,
    /// Фаза лёгкого дрожания (у каждой лампы своя).
    phase: f32,
}

/// Маркер колбы мигающей лампы (материал уникальный, дёргаем base_color).
#[derive(Component)]
struct LampBulb;

/// Маркер гало мигающей лампы (материал уникальный, дёргаем прозрачность).
#[derive(Component)]
struct LampGlow;

/// Маркер спрайта, который каждый кадр поворачивается к камере (гало ламп).
#[derive(Component)]
struct Billboard;

/// Пылинка в свете лампы: дрейфует внутри бокса вокруг якоря.
#[derive(Component)]
struct DustMote {
    vel: Vec3,
    anchor: Vec3,
    half: Vec3,
}

/// Кадры зерна плёнки: сущности полноэкранных нод + таймер переключения.
#[derive(Resource)]
struct GrainFrames {
    frames: Vec<Entity>,
    idx: usize,
    timer: Timer,
}

/// Разновидность светильника.
enum LampKind {
    /// Разбитый: тёмная колба, висит криво, света нет.
    Broken,
    /// Рабочий: ровный тёплый свет.
    Steady,
    /// Неисправный: свет дёргается (уникальные материалы колбы и гало).
    Flicker,
}

/// Общий набор мешей и материалов светильников (дешёвые Handle-клоны).
#[derive(Clone)]
struct LampKit {
    cord_mesh: Handle<Mesh>,
    shade_mesh: Handle<Mesh>,
    bulb_mesh: Handle<Mesh>,
    glow_mesh: Handle<Mesh>,
    cord_mat: Handle<StandardMaterial>,
    shade_mat: Handle<StandardMaterial>,
    bulb_on_mat: Handle<StandardMaterial>,
    bulb_off_mat: Handle<StandardMaterial>,
    glow_on_mat: Handle<StandardMaterial>,
    glow_tex: Handle<Image>,
    mote_mesh: Handle<Mesh>,
    mote_mat: Handle<StandardMaterial>,
}

// ---------------------------------------------------------------------------
// Плагин
// ---------------------------------------------------------------------------

/// Системы атмосферы: мерцание ламп, billboard-гало, пыль, зерно плёнки.
pub struct AtmospherePlugin;

impl Plugin for AtmospherePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (flicker_lamps, billboard_glows, drift_dust, tick_film_grain)
                .run_if(in_state(AppState::InGame)),
        );
    }
}

// ---------------------------------------------------------------------------
// Спавн ламп
// ---------------------------------------------------------------------------

/// Расстановка светильников по лабиринту (`true` в карте - стена).
///
/// Сетка с шагом [`LAMP_STEP`], часть ламп разбита или мигает, плюс
/// [`DARK_ZONE_COUNT`] тёмные зоны, где света нет вообще. Спавн игрока
/// (клетка (1, 1)) всегда освещён. Вызывается из `generate_level`.
pub(crate) fn spawn_lamps(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    images: &mut ResMut<Assets<Image>>,
    maze: &[Vec<bool>],
) {
    let mut rng = rand::thread_rng();
    let map_w = maze.first().map(Vec::len).unwrap_or(0);
    let map_h = maze.len();

    // Тёмные зоны: прямоугольники клеток, где ламп нет вообще.
    // x0 >= 3, поэтому спавн игрока (1, 1) всегда вне тёмных зон.
    let mut dark_zones = Vec::with_capacity(DARK_ZONE_COUNT);
    for _ in 0..DARK_ZONE_COUNT {
        let zw = rng.gen_range(4..=6) as i32;
        let zh = rng.gen_range(3..=5) as i32;
        let x0 = rng.gen_range(3..(map_w as i32 - zw - 1).max(4));
        let y0 = rng.gen_range(1..(map_h as i32 - zh - 1).max(2));
        dark_zones.push((x0, y0, x0 + zw, y0 + zh));
    }
    let in_dark_zone = |cx: i32, cy: i32| {
        dark_zones
            .iter()
            .any(|&(x0, y0, x1, y1)| cx >= x0 && cx < x1 && cy >= y0 && cy < y1)
    };

    // Общие меши и материалы светильников.
    let glow_tex = images.add(crate::textures::build_glow_texture());
    let kit = LampKit {
        cord_mesh: meshes.add(Cuboid::new(0.05, CORD_LEN, 0.05)),
        shade_mesh: meshes.add(Cuboid::new(0.55, 0.16, 0.55)),
        bulb_mesh: meshes.add(Sphere::new(0.09).mesh().uv(16, 12)),
        glow_mesh: meshes.add(Plane3d::default().mesh()),
        cord_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(0.02, 0.02, 0.025),
            perceptual_roughness: 0.9,
            ..default()
        }),
        shade_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(0.10, 0.10, 0.11),
            perceptual_roughness: 0.55,
            metallic: 0.6,
            ..default()
        }),
        bulb_on_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(LAMP_BULB_RGB.0, LAMP_BULB_RGB.1, LAMP_BULB_RGB.2),
            unlit: true,
            ..default()
        }),
        bulb_off_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(0.07, 0.07, 0.08),
            unlit: true,
            ..default()
        }),
        glow_on_mat: materials.add(glow_material(glow_tex.clone())),
        glow_tex,
        mote_mesh: meshes.add(Sphere::new(0.012).mesh().uv(6, 4)),
        mote_mat: materials.add(StandardMaterial {
            base_color: Color::srgba(1.0, 0.9, 0.75, 0.10),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            ..default()
        }),
    };

    let mut lamp_count = 0u32;
    let mut dark_count = 0u32;
    for cy in (1..map_h).step_by(LAMP_STEP) {
        for cx in (1..map_w).step_by(LAMP_STEP) {
            if maze.get(cy).and_then(|row| row.get(cx)) != Some(&false) {
                continue; // не проход - светильник не ставим
            }
            let (x, z) = crate::cell_center(cx, cy);
            let base = Vec3::new(x, 0.0, z);
            if in_dark_zone(cx as i32, cy as i32) {
                dark_count += 1;
                // В тёмной зоне света нет; иногда висит разбитый плафон.
                if rng.gen_bool(DARK_ZONE_FIXTURE_CHANCE) {
                    spawn_lamp(commands, &mut *materials, &kit, base, LampKind::Broken);
                }
                continue;
            }
            lamp_count += 1;
            let kind = if rng.gen_bool(BROKEN_CHANCE) {
                LampKind::Broken
            } else if rng.gen_bool(FLICKER_CHANCE) {
                LampKind::Flicker
            } else {
                LampKind::Steady
            };
            spawn_lamp(commands, &mut *materials, &kit, base, kind);
        }
    }
    info!(
        "Lamps: {lamp_count} spots ({dark_count} inside dark zones, no light there)"
    );
}

/// Материал гало: тёплый оттенок, радиальная текстура, аддитивный блендинг.
fn glow_material(tex: Handle<Image>) -> StandardMaterial {
    StandardMaterial {
        base_color: Color::srgba(
            LAMP_GLOW_RGB.0,
            LAMP_GLOW_RGB.1,
            LAMP_GLOW_RGB.2,
            LAMP_GLOW_ALPHA,
        ),
        base_color_texture: Some(tex),
        unlit: true,
        alpha_mode: AlphaMode::Add,
        cull_mode: None,
        ..default()
    }
}

/// Один светильник: шнур + плафон + колба, у рабочих - свет, гало и пыль.
fn spawn_lamp(
    commands: &mut Commands,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    kit: &LampKit,
    base: Vec3,
    kind: LampKind,
) {
    let broken = matches!(kind, LampKind::Broken);
    let flicker = matches!(kind, LampKind::Flicker);
    // Разбитый плафон висит криво.
    let tilt = if broken {
        Quat::from_euler(EulerRot::XYZ, 0.14, 0.0, 0.38)
    } else {
        Quat::IDENTITY
    };
    let bulb_mat = match kind {
        LampKind::Broken => kit.bulb_off_mat.clone(),
        LampKind::Steady => kit.bulb_on_mat.clone(),
        // Мигающей лампе - уникальный материал колбы.
        LampKind::Flicker => materials.add(StandardMaterial {
            base_color: Color::srgb(LAMP_BULB_RGB.0, LAMP_BULB_RGB.1, LAMP_BULB_RGB.2),
            unlit: true,
            ..default()
        }),
    };

    let mut root = commands.spawn((
        Transform::from_translation(base),
        Visibility::default(),
        StateScoped(AppState::InGame),
    ));
    if flicker {
        root.insert(FlickerLamp {
            base_intensity: LAMP_INTENSITY,
            bulb_rgb: Vec3::new(LAMP_BULB_RGB.0, LAMP_BULB_RGB.1, LAMP_BULB_RGB.2),
            glow_rgb: Vec3::new(LAMP_GLOW_RGB.0, LAMP_GLOW_RGB.1, LAMP_GLOW_RGB.2),
            glow_alpha: LAMP_GLOW_ALPHA,
            level: 1.0,
            target: 1.0,
            retarget: 0.0,
            phase: rand::thread_rng().gen_range(0.0..std::f32::consts::TAU),
        });
    }
    root.with_children(|root| {
        root.spawn((
            Mesh3d(kit.cord_mesh.clone()),
            MeshMaterial3d(kit.cord_mat.clone()),
            Transform::from_xyz(0.0, CORD_Y, 0.0),
            StateScoped(AppState::InGame),
        ));
        root.spawn((
            Mesh3d(kit.shade_mesh.clone()),
            MeshMaterial3d(kit.shade_mat.clone()),
            Transform {
                translation: Vec3::new(0.0, SHADE_Y, 0.0),
                rotation: tilt,
                ..default()
            },
            StateScoped(AppState::InGame),
        ));
        let mut bulb = root.spawn((
            Mesh3d(kit.bulb_mesh.clone()),
            MeshMaterial3d(bulb_mat),
            Transform {
                translation: Vec3::new(0.0, BULB_Y, 0.0),
                rotation: tilt,
                ..default()
            },
            StateScoped(AppState::InGame),
        ));
        if flicker {
            bulb.insert(LampBulb);
        }
        if !broken {
            root.spawn((
                PointLight {
                    color: Color::srgb(LAMP_LIGHT_RGB.0, LAMP_LIGHT_RGB.1, LAMP_LIGHT_RGB.2),
                    intensity: LAMP_INTENSITY,
                    range: LAMP_RANGE,
                    ..default()
                },
                Transform::from_xyz(0.0, LIGHT_Y, 0.0),
                StateScoped(AppState::InGame),
            ));
            // Мигающей лампе - уникальный материал гало.
            let glow_mat = if flicker {
                materials.add(glow_material(kit.glow_tex.clone()))
            } else {
                kit.glow_on_mat.clone()
            };
            let mut glow = root.spawn((
                Mesh3d(kit.glow_mesh.clone()),
                MeshMaterial3d(glow_mat),
                Transform {
                    translation: Vec3::new(0.0, BULB_Y, 0.0),
                    rotation: Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
                    scale: Vec3::splat(LAMP_GLOW_SIZE),
                },
                Billboard,
                StateScoped(AppState::InGame),
            ));
            if flicker {
                glow.insert(LampGlow);
            }
        }
    });

    if !broken {
        spawn_dust(commands, kit, base);
    }
}

/// Пылинки, дрейфующие в свете рабочей лампы.
fn spawn_dust(commands: &mut Commands, kit: &LampKit, base: Vec3) {
    let mut rng = rand::thread_rng();
    let anchor = base + Vec3::new(0.0, 1.7, 0.0);
    for _ in 0..DUST_PER_LAMP {
        commands.spawn((
            Mesh3d(kit.mote_mesh.clone()),
            MeshMaterial3d(kit.mote_mat.clone()),
            Transform::from_translation(
                anchor + Vec3::new(
                    rng.gen_range(-1.4..1.4),
                    rng.gen_range(-0.9..0.9),
                    rng.gen_range(-1.4..1.4),
                ),
            ),
            DustMote {
                vel: Vec3::new(
                    rng.gen_range(-0.06..0.06),
                    rng.gen_range(-0.03..0.01),
                    rng.gen_range(-0.06..0.06),
                ),
                anchor,
                half: Vec3::new(1.5, 1.0, 1.5),
            },
            StateScoped(AppState::InGame),
        ));
    }
}

// ---------------------------------------------------------------------------
// Кино-оверлеи: виньетка и зерно плёнки
// ---------------------------------------------------------------------------

/// Постоянные оверлеи поверх HUD: тёмная виньетка по краям + зерно плёнки.
/// Зерно - несколько предрасчитанных кадров, видимый переключается таймером.
/// Вызывается из `setup_hud`.
pub(crate) fn spawn_cinematic_overlays(
    commands: &mut Commands,
    images: &mut ResMut<Assets<Image>>,
) {
    commands.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        ImageNode::new(images.add(crate::textures::build_vignette_texture())),
        StateScoped(AppState::InGame),
    ));
    let mut frames = Vec::with_capacity(GRAIN_FRAMES);
    for i in 0..GRAIN_FRAMES {
        let entity = commands
            .spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
                ImageNode::new(images.add(crate::textures::build_grain_frame(i as i32))),
                Visibility::Hidden,
                StateScoped(AppState::InGame),
            ))
            .id();
        frames.push(entity);
    }
    commands.entity(frames[0]).insert(Visibility::Visible);
    commands.insert_resource(GrainFrames {
        frames,
        idx: 0,
        timer: Timer::from_seconds(GRAIN_FRAME_SECS, TimerMode::Repeating),
    });
}

// ---------------------------------------------------------------------------
// Системы
// ---------------------------------------------------------------------------

/// Мерцание неисправных ламп: в основном горит, иногда просадки и редкие
/// полные отключения. Дёргается свет, колба и гало - синхронно.
fn flicker_lamps(
    time: Res<Time>,
    mut roots: Query<(&mut FlickerLamp, &Children)>,
    mut lights: Query<&mut PointLight>,
    bulb_mats: Query<&MeshMaterial3d<StandardMaterial>, With<LampBulb>>,
    glow_mats: Query<&MeshMaterial3d<StandardMaterial>, With<LampGlow>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut rng = rand::thread_rng();
    let dt = time.delta_secs();
    let t = time.elapsed_secs();
    for (mut lamp, children) in &mut roots {
        // Случайные переключения цели: 8% - полное отключение,
        // 27% - просадка, иначе - полный свет.
        lamp.retarget -= dt;
        if lamp.retarget <= 0.0 {
            lamp.retarget = rng.gen_range(0.03..0.22);
            let roll: f32 = rng.gen();
            lamp.target = if roll < 0.08 {
                0.0
            } else if roll < 0.35 {
                rng.gen_range(0.08..0.55)
            } else {
                1.0
            };
        }
        // Резкий подход к цели (люминесцентный «щелчок») + лёгкое дрожание.
        lamp.level += (lamp.target - lamp.level) * (22.0 * dt).min(1.0);
        if (lamp.target - lamp.level).abs() < 0.02 {
            lamp.level = lamp.target;
        }
        let shimmer = 0.96 + 0.04 * (t * 43.0 + lamp.phase).sin();
        let m = (lamp.level * shimmer).clamp(0.0, 1.0);
        for child in children.iter() {
            if let Ok(mut light) = lights.get_mut(child) {
                light.intensity = lamp.base_intensity * m;
            }
            if let Ok(bulb) = bulb_mats.get(child) {
                if let Some(mat) = materials.get_mut(&bulb.0) {
                    mat.base_color = Color::srgb(
                        lamp.bulb_rgb.x * m,
                        lamp.bulb_rgb.y * m,
                        lamp.bulb_rgb.z * m,
                    );
                }
            }
            if let Ok(glow) = glow_mats.get(child) {
                if let Some(mat) = materials.get_mut(&glow.0) {
                    mat.base_color = Color::srgba(
                        lamp.glow_rgb.x * m,
                        lamp.glow_rgb.y * m,
                        lamp.glow_rgb.z * m,
                        lamp.glow_alpha * m,
                    );
                }
            }
        }
    }
}

/// Гало ламп всегда повёрнуты к игроку (billboard).
fn billboard_glows(
    cameras: Query<&Transform, (With<Player>, Without<Billboard>)>,
    mut glows: Query<&mut Transform, (With<Billboard>, Without<Player>)>,
) {
    let Some(cam) = cameras.iter().next() else {
        return;
    };
    // Квад Plane3d смотрит +Y; доворачиваем его нормаль на +Z камеры.
    let tilt = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
    let rotation = cam.rotation * tilt;
    for mut transform in &mut glows {
        transform.rotation = rotation;
    }
}

/// Медленный дрейф пылинок внутри бокса вокруг якоря (с заворотом у границ).
fn drift_dust(time: Res<Time>, mut motes: Query<(&mut Transform, &DustMote)>) {
    let dt = time.delta_secs();
    for (mut transform, mote) in &mut motes {
        let p = &mut transform.translation;
        p.x += mote.vel.x * dt;
        p.y += mote.vel.y * dt;
        p.z += mote.vel.z * dt;
        wrap_axis(&mut p.x, mote.anchor.x, mote.half.x);
        wrap_axis(&mut p.y, mote.anchor.y, mote.half.y);
        wrap_axis(&mut p.z, mote.anchor.z, mote.half.z);
    }
}

/// Заворот координаты внутрь отрезка [anchor - half, anchor + half].
fn wrap_axis(coord: &mut f32, anchor: f32, half: f32) {
    if *coord > anchor + half {
        *coord = anchor - half;
    } else if *coord < anchor - half {
        *coord = anchor + half;
    }
}

/// Переключение кадров зерна плёнки (~11 кадров/с).
fn tick_film_grain(
    time: Res<Time>,
    grain: Option<ResMut<GrainFrames>>,
    mut visibility: Query<&mut Visibility>,
) {
    let Some(mut grain) = grain else {
        return;
    };
    if grain.frames.is_empty() {
        return;
    }
    grain.timer.tick(time.delta());
    if !grain.timer.just_finished() {
        return;
    }
    grain.idx = (grain.idx + 1) % grain.frames.len();
    for (i, entity) in grain.frames.iter().enumerate() {
        if let Ok(mut vis) = visibility.get_mut(*entity) {
            *vis = if i == grain.idx {
                Visibility::Visible
            } else {
                Visibility::Hidden
            };
        }
    }
}
