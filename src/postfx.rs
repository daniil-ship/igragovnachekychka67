//! Постобработка: WGSL-шейдер зерна плёнки и виньетки + F11.
//!
//! Полноэкранный проход после тонемаппинга (по официальному примеру Bevy
//! `custom_post_processing` для 0.16): зерно дрожит каждый кадр, виньетка
//! сгущается пропорционально безумию. Настройки меню (`film_grain`,
//! `vignette`) едут в шейдер юниформой. Эффект висит только на 3D-камере
//! актов (компонент [`HorrorPostFx`]) - меню и финалы не трогаем.

use bevy::{
    core_pipeline::{
        core_3d::graph::{Core3d, Node3d},
        fullscreen_vertex_shader::fullscreen_shader_vertex_state,
    },
    ecs::query::QueryItem,
    prelude::*,
    render::{
        extract_component::{
            ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin,
            UniformComponentPlugin,
        },
        render_graph::{
            NodeRunError, RenderGraphApp, RenderGraphContext, RenderLabel, ViewNode, ViewNodeRunner,
        },
        render_resource::{
            binding_types::{sampler, texture_2d, uniform_buffer},
            *,
        },
        renderer::{RenderContext, RenderDevice},
        view::ViewTarget,
        RenderApp,
    },
};
use bevy::window::{MonitorSelection, WindowMode};

/// Путь шейдера внутри `assets/`.
const SHADER_ASSET_PATH: &str = "shaders/horror_postfx.wgsl";

// ---------------------------------------------------------------------------
// Плагин
// ---------------------------------------------------------------------------

/// Регистрирует WGSL-проход, юниформу на камере и переключение F11.
pub struct PostFxPlugin;

impl Plugin for PostFxPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            ExtractComponentPlugin::<HorrorPostFx>::default(),
            UniformComponentPlugin::<HorrorPostFx>::default(),
        ))
        .add_systems(Update, update_postfx_uniforms.run_if(crate::in_act))
        .add_systems(Update, fullscreen_toggle);

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_render_graph_node::<ViewNodeRunner<HorrorPostFxNode>>(Core3d, HorrorPostFxLabel)
            .add_render_graph_edges(
                Core3d,
                (
                    Node3d::Tonemapping,
                    HorrorPostFxLabel,
                    Node3d::EndMainPassPostProcessing,
                ),
            );
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app.init_resource::<HorrorPostFxPipeline>();
    }
}

// ---------------------------------------------------------------------------
// Юниформа на камере (main world -> render world -> GPU)
// ---------------------------------------------------------------------------

/// Настройки хоррор-постобработки на камере. Живут в main world,
/// каждый кадр извлекаются в render world и загружаются в GPU.
#[derive(Component, Default, Clone, Copy, ExtractComponent, ShaderType)]
pub struct HorrorPostFx {
    /// Секунды (дрожание зерна, дыхание виньетки).
    pub time: f32,
    /// Безумие 0..1 (глубина виньетки и сила зерна).
    pub insanity: f32,
    /// Зерно вкл/выкл (настройка меню), 1.0/0.0.
    pub grain_on: f32,
    /// Виньетка вкл/выкл (настройка меню), 1.0/0.0.
    pub vignette_on: f32,
}

/// Обновление юниформы из безумия игрока и настроек меню.
fn update_postfx_uniforms(
    time: Res<Time>,
    settings: Res<crate::GameSettings>,
    insanity_query: Query<&crate::hallucinations::Insanity, With<crate::player::Player>>,
    mut fx_query: Query<&mut HorrorPostFx>,
) {
    let madness = insanity_query
        .iter()
        .next()
        .map(|insanity| (insanity.0 / 100.0).clamp(0.0, 1.0))
        .unwrap_or(0.0);
    for mut fx in &mut fx_query {
        fx.time = time.elapsed_secs();
        fx.insanity = madness;
        fx.grain_on = if settings.film_grain { 1.0 } else { 0.0 };
        fx.vignette_on = if settings.vignette { 1.0 } else { 0.0 };
    }
}

// ---------------------------------------------------------------------------
// F11: полноэкранный режим
// ---------------------------------------------------------------------------

/// F11: переключение между окном и полноэкранным режимом (везде, даже в меню).
fn fullscreen_toggle(
    keys: Res<ButtonInput<KeyCode>>,
    mut windows: Query<&mut Window>,
) {
    if !keys.just_pressed(KeyCode::F11) {
        return;
    }
    for mut window in &mut windows {
        window.mode = match window.mode {
            WindowMode::Windowed => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
            _ => WindowMode::Windowed,
        };
        info!("Fullscreen: {:?}", window.mode);
    }
}

// ---------------------------------------------------------------------------
// Render-граф: нода и пайплайн (render world)
// ---------------------------------------------------------------------------

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct HorrorPostFxLabel;

#[derive(Default)]
struct HorrorPostFxNode;

impl ViewNode for HorrorPostFxNode {
    type ViewQuery = (
        &'static ViewTarget,
        // Нода работает только на камерах с компонентом настроек.
        &'static HorrorPostFx,
        // Индекс юниформы этого вида (камер с эффектом может быть несколько).
        &'static DynamicUniformIndex<HorrorPostFx>,
    );

    fn run(
        &self,
        _graph: &mut RenderGraphContext,
        render_context: &mut RenderContext,
        (view_target, _settings, settings_index): QueryItem<Self::ViewQuery>,
        world: &World,
    ) -> Result<(), NodeRunError> {
        let pipeline = world.resource::<HorrorPostFxPipeline>();
        let pipeline_cache = world.resource::<PipelineCache>();
        let Some(render_pipeline) = pipeline_cache.get_render_pipeline(pipeline.pipeline_id)
        else {
            return Ok(());
        };
        let uniforms = world.resource::<ComponentUniforms<HorrorPostFx>>();
        let Some(settings_binding) = uniforms.uniforms().binding() else {
            return Ok(());
        };
        // `source` - текущий главный кадр; писать ОБЯЗАНЫ в `destination`,
        // иначе главная текстура вида потеряется.
        let post_process = view_target.post_process_write();
        // Bind-группа создаётся каждый кадр: source/destination чередуются.
        let bind_group = render_context.render_device().create_bind_group(
            "horror_postfx_bind_group",
            &pipeline.layout,
            &BindGroupEntries::sequential((
                post_process.source,
                &pipeline.sampler,
                settings_binding.clone(),
            )),
        );
        let mut render_pass =
            render_context.begin_tracked_render_pass(RenderPassDescriptor {
                label: Some("horror_postfx_pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: post_process.destination,
                    resolve_target: None,
                    ops: Operations::default(),
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        render_pass.set_render_pipeline(render_pipeline);
        render_pass.set_bind_group(0, &bind_group, &[settings_index.index()]);
        render_pass.draw(0..3, 0..1);
        Ok(())
    }
}

/// Глобальные данные пайплайна: создаются один раз на старте.
#[derive(Resource)]
struct HorrorPostFxPipeline {
    layout: BindGroupLayout,
    sampler: Sampler,
    pipeline_id: CachedRenderPipelineId,
}

impl FromWorld for HorrorPostFxPipeline {
    fn from_world(world: &mut World) -> Self {
        let render_device = world.resource::<RenderDevice>();
        let layout = render_device.create_bind_group_layout(
            "horror_postfx_bind_group_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                    uniform_buffer::<HorrorPostFx>(true),
                ),
            ),
        );
        let sampler = render_device.create_sampler(&SamplerDescriptor::default());
        let shader: Handle<Shader> = world
            .resource::<AssetServer>()
            .load(SHADER_ASSET_PATH);
        let pipeline_id = world
            .resource_mut::<PipelineCache>()
            .queue_render_pipeline(RenderPipelineDescriptor {
                label: Some("horror_postfx_pipeline".into()),
                layout: vec![layout.clone()],
                vertex: fullscreen_shader_vertex_state(),
                fragment: Some(FragmentState {
                    shader,
                    shader_defs: vec![],
                    entry_point: "fragment".into(),
                    targets: vec![Some(ColorTargetState {
                        format: TextureFormat::bevy_default(),
                        blend: None,
                        write_mask: ColorWrites::ALL,
                    })],
                }),
                primitive: PrimitiveState::default(),
                depth_stencil: None,
                multisample: MultisampleState::default(),
                push_constant_ranges: vec![],
                zero_initialize_workgroup_memory: false,
            });
        Self {
            layout,
            sampler,
            pipeline_id,
        }
    }
}
