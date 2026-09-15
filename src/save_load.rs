//! Сохранения: F5 пишет JSON, F9 читает и восстанавливает забег.
//!
//! [`SaveData`] хранит акт, сид лабиринта, позицию и взгляд игрока, батарею,
//! безумие, счётчики фрагментов и реликвий, услышанные кассеты и остаток
//! реактора. Загрузка выставляет сид (тот же лабиринт и раскладка один в один),
//! переключает состояние и накатывает данные поверх свежей генерации.
//! Если точка спавна вдруг внутри стены - игрок сдвигается на свободное место.

use std::fs;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::hallucinations::Insanity;
use crate::interaction::{AudioCassette, QuestItem, QuestsDone};
use crate::player::{Flashlight, Player};

/// Файл сохранения рядом с `assets/`.
const SAVE_PATH: &str = "save.json";

// ---------------------------------------------------------------------------
// Плагин
// ---------------------------------------------------------------------------

/// Регистрирует сохранение (F5, только в актах), загрузку (F9, везде)
/// и накат загрузки поверх генерации.
pub struct SaveLoadPlugin;

impl Plugin for SaveLoadPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, save_game.run_if(crate::in_act))
            .add_systems(Update, load_game)
            .add_systems(Update, apply_pending_load.run_if(crate::in_act));
    }
}

// ---------------------------------------------------------------------------
// Данные и загрузка
// ---------------------------------------------------------------------------

/// Снимок забега для JSON.
#[derive(Serialize, Deserialize)]
struct SaveData {
    /// Акт: 1, 2 или 3.
    act: u8,
    /// Сид лабиринта и раскладки (тот же мир один в один).
    seed: u64,
    /// Позиция игрока.
    pos: [f32; 3],
    /// Взгляд игрока.
    yaw: f32,
    pitch: f32,
    /// Заряд батареи и состояние фонаря.
    battery: f32,
    lamp_on: bool,
    /// Безумие 0..100.
    insanity: f32,
    /// Собранные фрагменты и реликвии.
    collected: u32,
    quest_items: u32,
    /// Глобальные индексы собранных реликвий (какие именно).
    quests_done: Vec<usize>,
    /// Индексы лора услышанных кассет.
    cassettes_heard: Vec<usize>,
    /// Остаток реактора (только Акт 3).
    reactor_left: Option<f32>,
}

/// Загрузка, ожидающая генерации уровня (накатывается в `apply_pending_load`).
#[derive(Resource)]
struct PendingLoad(SaveData);

// ---------------------------------------------------------------------------
// Системы
// ---------------------------------------------------------------------------

/// F5: снять состояние забега и записать в `save.json`.
fn save_game(
    keys: Res<ButtonInput<KeyCode>>,
    players: Query<(&Transform, &Player, &Insanity)>,
    lamps: Query<&Flashlight>,
    tapes: Query<&AudioCassette>,
    progress: Res<crate::GameProgress>,
    quests: Res<QuestsDone>,
    seed: Res<crate::CurrentLevelSeed>,
    reactor: Option<Res<crate::ReactorTimer>>,
    game_state: Res<State<crate::GameState>>,
) {
    if !keys.just_pressed(KeyCode::F5) {
        return;
    }
    let act = match *game_state.get() {
        crate::GameState::Act1_TheDescent => 1,
        crate::GameState::Act2_TheInsanity => 2,
        crate::GameState::Act3_TheReactor => 3,
        _ => return,
    };
    let Some((transform, player, insanity)) = players.iter().next() else {
        return;
    };
    let lamp = lamps.iter().next();
    let data = SaveData {
        act,
        seed: seed.0,
        pos: [
            transform.translation.x,
            transform.translation.y,
            transform.translation.z,
        ],
        yaw: player.yaw,
        pitch: player.pitch,
        battery: lamp.map(|lamp| lamp.battery).unwrap_or(0.0),
        lamp_on: lamp.map(|lamp| lamp.is_on).unwrap_or(false),
        insanity: insanity.0,
        collected: progress.collected,
        quest_items: progress.quest_items,
        quests_done: quests.0.clone(),
        cassettes_heard: tapes
            .iter()
            .filter(|tape| tape.was_played)
            .map(|tape| tape.lore_index)
            .collect(),
        reactor_left: reactor.map(|reactor| reactor.time_left),
    };
    match serde_json::to_string_pretty(&data) {
        Ok(json) => match fs::write(SAVE_PATH, json) {
            Ok(()) => info!("Game saved to {SAVE_PATH} (act {act})"),
            Err(e) => warn!("Failed to write {SAVE_PATH}: {e}"),
        },
        Err(e) => warn!("Failed to serialize save: {e}"),
    }
}

/// F9: прочитать `save.json`, выставить сид и переключить состояние.
/// Данные накатаются в `apply_pending_load`, когда генерация даст игрока.
fn load_game(
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    mut next_state: ResMut<NextState<crate::GameState>>,
) {
    if !keys.just_pressed(KeyCode::F9) {
        return;
    }
    let text = match fs::read_to_string(SAVE_PATH) {
        Ok(text) => text,
        Err(_) => {
            warn!("No {SAVE_PATH} yet - press F5 to save first");
            return;
        }
    };
    let data: SaveData = match serde_json::from_str(&text) {
        Ok(data) => data,
        Err(e) => {
            warn!("Broken {SAVE_PATH}: {e}");
            return;
        }
    };
    let act = match data.act {
        1 => crate::GameState::Act1_TheDescent,
        2 => crate::GameState::Act2_TheInsanity,
        3 => crate::GameState::Act3_TheReactor,
        _ => {
            warn!("Save has unknown act {}", data.act);
            return;
        }
    };
    // Тот же сид = тот же лабиринт и раскладка; остальное доложит apply.
    commands.insert_resource(crate::LevelSeedOverride(Some(data.seed)));
    commands.insert_resource(PendingLoad(data));
    next_state.set(act);
    info!("Save loaded - entering act");
}

/// Накат загрузки: ждём игрока от генерации, телепортируем, восстанавливаем
/// батарею/безумие/счётчики, убираем уже собранное, чиним кассеты и реактор.
fn apply_pending_load(
    mut commands: Commands,
    pending: Option<Res<PendingLoad>>,
    mut players: Query<(&mut Transform, &mut Player, &mut Insanity)>,
    mut lamps: Query<&mut Flashlight>,
    mut tapes: Query<&mut AudioCassette>,
    fragments: Query<Entity, With<crate::EchoFragment>>,
    quest_entities: Query<(Entity, &QuestItem)>,
    mut progress: ResMut<crate::GameProgress>,
    mut reactor: Option<ResMut<crate::ReactorTimer>>,
    colliders: Res<crate::LevelColliders>,
) {
    let Some(pending) = pending else {
        return;
    };
    // Игрок появляется только после генерации - ждём её.
    let Some((mut transform, mut player, mut insanity)) = players.iter_mut().next() else {
        return;
    };
    let data = &pending.0;
    let pos = ensure_free_spot(
        Vec3::new(data.pos[0], data.pos[1], data.pos[2]),
        &colliders.walls,
    );
    transform.translation = pos;
    transform.rotation = Quat::from_euler(EulerRot::YXZ, data.yaw, data.pitch, 0.0);
    player.yaw = data.yaw;
    player.pitch = data.pitch;
    insanity.0 = data.insanity;
    for mut lamp in &mut lamps {
        lamp.battery = data.battery;
        lamp.is_on = data.lamp_on;
    }
    progress.collected = data.collected.min(progress.total);
    progress.quest_items = data.quest_items;
    // Уже собранного в мире быть не должно: фрагменты - любые лишние...
    let keep_fragments = progress.total.saturating_sub(progress.collected) as usize;
    let have_fragments = fragments.iter().count();
    for entity in fragments
        .iter()
        .take(have_fragments.saturating_sub(keep_fragments))
    {
        commands.entity(entity).despawn();
    }
    // ...реликвии - ровно те индексы, что уже собраны.
    for (entity, item) in &quest_entities {
        if data.quests_done.contains(&item.index) {
            commands.entity(entity).despawn();
        }
    }
    for mut tape in &mut tapes {
        if data.cassettes_heard.contains(&tape.lore_index) {
            tape.was_played = true;
        }
    }
    if let (Some(mut reactor), Some(left)) = (reactor, data.reactor_left) {
        reactor.time_left = left;
    }
    commands.remove_resource::<PendingLoad>();
    info!(
        "Save applied: act {}, keepsakes {}/{}",
        data.act,
        data.quest_items,
        crate::interaction::QUEST_COUNT
    );
}

/// Точка внутри стены - ищем свободное место рядом, иначе спавн (1, 1).
/// Страховка на случай загрузки в тот же акт без регенерации.
fn ensure_free_spot(pos: Vec3, walls: &[crate::WallAabb]) -> Vec3 {
    let free = |p: Vec3| {
        !walls.iter().any(|w| {
            p.x + 0.4 > w.min_x && p.x - 0.4 < w.max_x && p.z + 0.4 > w.min_z && p.z - 0.4 < w.max_z
        })
    };
    if free(pos) {
        return pos;
    }
    for r in 1..=10 {
        for k in 0..8 {
            let a = k as f32 * std::f32::consts::FRAC_PI_4 + r as f32;
            let p = Vec3::new(
                pos.x + a.cos() * r as f32,
                pos.y,
                pos.z + a.sin() * r as f32,
            );
            if free(p) {
                return p;
            }
        }
    }
    let (x, z) = crate::cell_center(1, 1);
    Vec3::new(x, pos.y, z)
}
