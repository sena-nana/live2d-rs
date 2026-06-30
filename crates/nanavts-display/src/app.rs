use crate::{
    http::{spawn_http_server, DisplayState},
    live2d::{Live2dGpuRenderer, Live2dScene, Live2dView},
    preview::PreviewUniform,
    replay::{load_replay_session, post_replay_session},
    session,
};
use std::{
    path::PathBuf,
    sync::{Arc, RwLock},
    time::Instant,
};
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

const MIN_PREVIEW_ZOOM: f32 = 0.5;
const MAX_PREVIEW_ZOOM: f32 = 6.0;
const DRAG_THRESHOLD_PX: f64 = 3.0;

pub fn run_from_env_args() {
    let mode = match StartupMode::parse(std::env::args().skip(1)) {
        Ok(mode) => mode,
        Err(err) => {
            eprintln!("{err}");
            eprintln!("{}", StartupMode::usage());
            std::process::exit(2);
        }
    };

    let addr = std::env::var("NANAVTS_DISPLAY_ADDR").unwrap_or_else(|_| "127.0.0.1:19676".into());
    let state = Arc::new(RwLock::new(DisplayState::new()));
    if let Err(err) = spawn_http_server(addr.clone(), state.clone()) {
        eprintln!("failed to start NanaVTS display HTTP server on {addr}: {err}");
        std::process::exit(1);
    }
    if let StartupMode::Replay(path) = mode {
        let session = match load_replay_session(path.as_deref()) {
            Ok(session) => session,
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
        };
        match post_replay_session(&addr, &session) {
            Ok(status) => {
                eprintln!("replayed NanaVTS display session to http://{addr}/session: {status}")
            }
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
        }
    }

    let event_loop = EventLoop::new().expect("create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = App {
        state,
        window: None,
        renderer: None,
        cursor_pos: None,
        view: PreviewView::default(),
        drag: None,
        start: Instant::now(),
    };
    event_loop
        .run_app(&mut app)
        .expect("run display event loop");
}

#[derive(Debug, Default, PartialEq, Eq)]
enum StartupMode {
    #[default]
    Preview,
    Replay(Option<PathBuf>),
}

impl StartupMode {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut args = args.into_iter();
        let Some(command) = args.next() else {
            return Ok(Self::Preview);
        };
        if command != "replay" {
            return Err(format!("unknown argument: {command}"));
        }
        let path = args.next().map(PathBuf::from);
        if let Some(extra) = args.next() {
            return Err(format!("unexpected argument: {extra}"));
        }
        Ok(Self::Replay(path))
    }

    fn usage() -> &'static str {
        "Usage: nanavts-display-wgpu [replay [session.json]]"
    }
}

struct App {
    state: Arc<RwLock<DisplayState>>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    cursor_pos: Option<(f64, f64)>,
    view: PreviewView,
    drag: Option<PreviewDrag>,
    start: Instant,
}

#[derive(Debug, Clone, Copy)]
struct PreviewView {
    offset_x: f32,
    offset_y: f32,
    zoom: f32,
}

impl Default for PreviewView {
    fn default() -> Self {
        Self {
            offset_x: 0.0,
            offset_y: 0.0,
            zoom: 1.0,
        }
    }
}

impl PreviewView {
    fn transform(self) -> [f32; 4] {
        [self.offset_x, self.offset_y, self.zoom, 0.0]
    }

    fn content_point_at(self, point: (f32, f32)) -> (f32, f32) {
        (
            (point.0 - 0.5 - self.offset_x) / self.zoom + 0.5,
            (point.1 - 0.5 - self.offset_y) / self.zoom + 0.5,
        )
    }

    fn zoom_at(&mut self, point: (f32, f32), factor: f32) {
        let previous = self.zoom;
        let next = (previous * factor).clamp(MIN_PREVIEW_ZOOM, MAX_PREVIEW_ZOOM);
        if (next - previous).abs() <= f32::EPSILON {
            return;
        }

        let content = self.content_point_at(point);
        self.zoom = next;
        self.offset_x = point.0 - 0.5 - (content.0 - 0.5) * next;
        self.offset_y = point.1 - 0.5 - (content.1 - 0.5) * next;
    }

    fn pan_by_pixels(&mut self, delta_x: f64, delta_y: f64, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        let aspect = size.width as f32 / size.height as f32;
        self.offset_x += (delta_x as f32 / size.width as f32) * aspect;
        self.offset_y += delta_y as f32 / size.height as f32;
    }
}

#[derive(Debug, Clone, Copy)]
struct PreviewDrag {
    start: (f64, f64),
    last: (f64, f64),
    dragged: bool,
}

impl PreviewDrag {
    fn new(position: (f64, f64)) -> Self {
        Self {
            start: position,
            last: position,
            dragged: false,
        }
    }

    fn update(&mut self, position: (f64, f64)) -> Option<(f64, f64)> {
        let delta = (position.0 - self.last.0, position.1 - self.last.1);
        self.last = position;

        let total_x = position.0 - self.start.0;
        let total_y = position.1 - self.start.1;
        if total_x.hypot(total_y) >= DRAG_THRESHOLD_PX {
            self.dragged = true;
        }

        self.dragged.then_some(delta)
    }

    fn is_click(self) -> bool {
        !self.dragged
    }
}

fn preview_point_from_cursor(position: (f64, f64), size: PhysicalSize<u32>) -> Option<(f32, f32)> {
    if size.width == 0 || size.height == 0 {
        return None;
    }

    let width = size.width as f32;
    let height = size.height as f32;
    let aspect = width / height;
    let x = ((position.0 as f32 / width) - 0.5) * aspect + 0.5;
    let y = position.1 as f32 / height;
    Some((x, y))
}

fn wheel_zoom_factor(delta: MouseScrollDelta) -> f32 {
    match delta {
        MouseScrollDelta::LineDelta(_, y) => 1.2_f32.powf(y),
        MouseScrollDelta::PixelDelta(position) => 1.0015_f32.powf(position.y as f32),
    }
}

fn is_picker_click(picker_active: bool, drag: Option<PreviewDrag>) -> bool {
    picker_active && drag.is_some_and(PreviewDrag::is_click)
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("NanaVTS Display")
                        .with_inner_size(PhysicalSize::new(960, 720)),
                )
                .expect("create display window"),
        );
        self.renderer = Some(pollster::block_on(Renderer::new(window.clone())));
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.window.as_ref().map(|window| window.id()) != Some(window_id) {
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size);
                }
            }
            WindowEvent::RedrawRequested => {
                if let (Some(window), Some(renderer)) = (&self.window, &mut self.renderer) {
                    let time = self.start.elapsed().as_secs_f32();
                    let (uniform, has_session, scene, target_art_mesh_ids) = {
                        let guard = self.state.read().expect("display state poisoned");
                        let picker_hovered = guard
                            .picker
                            .as_ref()
                            .and_then(|picker| picker.hovered.as_ref())
                            .is_some();
                        let uniform = PreviewUniform::from_session(
                            guard.session.as_ref(),
                            time,
                            renderer.size.width,
                            renderer.size.height,
                        )
                        .with_picker_hover(picker_hovered)
                        .with_view_transform(self.view.transform());
                        let target_art_mesh_ids = guard
                            .session
                            .as_ref()
                            .and_then(session::active_channel)
                            .map(|channel| channel.art_mesh_ids.clone())
                            .unwrap_or_default();
                        (
                            uniform,
                            guard.session.is_some(),
                            guard.scene.clone(),
                            target_art_mesh_ids,
                        )
                    };
                    if let Err(err) = renderer.render(
                        uniform,
                        has_session,
                        scene.as_ref(),
                        self.view.transform(),
                        target_art_mesh_ids,
                    ) {
                        eprintln!("display render failed: {err}");
                    }
                    window.request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let cursor = (position.x, position.y);
                self.cursor_pos = Some(cursor);
                if let Some(renderer) = &self.renderer {
                    if let Some(drag) = &mut self.drag {
                        if let Some((delta_x, delta_y)) = drag.update(cursor) {
                            self.view.pan_by_pixels(delta_x, delta_y, renderer.size);
                        }
                    }

                    let mut guard = self.state.write().expect("display state poisoned");
                    if guard.picker.is_some() {
                        guard.pick_artmesh_at(
                            position.x,
                            position.y,
                            renderer.size.width,
                            renderer.size.height,
                            false,
                        );
                        if let (Some(window), Some(label)) = (
                            &self.window,
                            guard
                                .picker
                                .as_ref()
                                .and_then(|picker| picker.hovered.as_ref()),
                        ) {
                            let title = guard
                                .picker_purpose_label()
                                .map(|purpose| {
                                    format!("NanaVTS Display - {purpose} - {}", label.label)
                                })
                                .unwrap_or_else(|| format!("NanaVTS Display - {}", label.label));
                            window.set_title(&title);
                        }
                    }
                }
            }
            WindowEvent::CursorLeft { .. } => {
                self.cursor_pos = None;
                self.drag = None;
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if let (Some(position), Some(renderer)) = (self.cursor_pos, &self.renderer) {
                    if let Some(point) = preview_point_from_cursor(position, renderer.size) {
                        self.view.zoom_at(point, wheel_zoom_factor(delta));
                    }
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(position) = self.cursor_pos {
                    self.drag = Some(PreviewDrag::new(position));
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                let drag = self.drag.take();
                if let (Some((x, y)), Some(renderer)) = (self.cursor_pos, &self.renderer) {
                    let mut guard = self.state.write().expect("display state poisoned");
                    if is_picker_click(guard.picker.is_some(), drag) {
                        guard.pick_artmesh_at(
                            x,
                            y,
                            renderer.size.width,
                            renderer.size.height,
                            true,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> StartupMode {
        StartupMode::parse(args.iter().map(|arg| arg.to_string())).expect("valid args")
    }

    #[test]
    fn parses_default_replay_entry() {
        assert_eq!(parse(&["replay"]), StartupMode::Replay(None));
    }

    #[test]
    fn parses_replay_session_path() {
        assert_eq!(
            parse(&["replay", "session.json"]),
            StartupMode::Replay(Some(PathBuf::from("session.json")))
        );
    }

    #[test]
    fn zoom_keeps_cursor_anchor_stable() {
        let mut view = PreviewView::default();
        let point = preview_point_from_cursor((800.0, 180.0), PhysicalSize::new(1000, 500))
            .expect("valid preview point");
        let before = view.content_point_at(point);

        view.zoom_at(point, 2.0);
        let after = view.content_point_at(point);

        assert_close(before.0, after.0);
        assert_close(before.1, after.1);
        assert_close(view.zoom, 2.0);
    }

    #[test]
    fn drag_pan_uses_aspect_correct_preview_units() {
        let mut view = PreviewView::default();

        view.pan_by_pixels(100.0, 50.0, PhysicalSize::new(1000, 500));

        assert_close(view.offset_x, 0.2);
        assert_close(view.offset_y, 0.1);
    }

    #[test]
    fn drag_threshold_separates_click_from_pan() {
        let mut drag = PreviewDrag::new((20.0, 20.0));

        assert!(drag.update((22.0, 21.0)).is_none());
        assert!(drag.is_click());

        assert_eq!(drag.update((24.0, 20.0)), Some((2.0, -1.0)));
        assert!(!drag.is_click());
    }

    #[test]
    fn picker_click_commits_only_in_picker_mode_without_dragging() {
        let click = PreviewDrag::new((0.0, 0.0));
        let dragged = PreviewDrag {
            start: (0.0, 0.0),
            last: (8.0, 0.0),
            dragged: true,
        };

        assert!(!is_picker_click(false, Some(click)));
        assert!(is_picker_click(true, Some(click)));
        assert!(!is_picker_click(true, Some(dragged)));
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.0001,
            "expected {actual} to be close to {expected}"
        );
    }
}

struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: PhysicalSize<u32>,
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    live2d_renderer: Live2dGpuRenderer,
}

impl Renderer {
    async fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window)
            .expect("create wgpu surface");
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("request wgpu adapter");
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("NanaVTS Display Device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .await
            .expect("request wgpu device");
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: caps.present_modes[0],
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("NanaVTS Display Preview Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("preview.wgsl").into()),
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("NanaVTS Preview Uniform"),
            size: std::mem::size_of::<PreviewUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("NanaVTS Preview Bind Group Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("NanaVTS Preview Bind Group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("NanaVTS Preview Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("NanaVTS Preview Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let live2d_renderer = Live2dGpuRenderer::new(&device, format);

        Self {
            surface,
            device,
            queue,
            config,
            size,
            pipeline,
            uniform_buffer,
            bind_group,
            live2d_renderer,
        }
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.size = size;
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
    }

    fn render(
        &mut self,
        uniform: PreviewUniform,
        has_session: bool,
        scene: Option<&Live2dScene>,
        transform: [f32; 4],
        target_art_mesh_ids: Vec<String>,
    ) -> Result<(), &'static str> {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                self.resize(self.size);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(())
            }
            wgpu::CurrentSurfaceTexture::Validation => return Err("surface validation failed"),
        };
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("NanaVTS Display Encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("NanaVTS Display Preview Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            if let Some(scene) = scene {
                self.live2d_renderer.render(
                    &self.device,
                    &self.queue,
                    &mut pass,
                    scene,
                    Live2dView {
                        transform,
                        width: self.size.width,
                        height: self.size.height,
                        effect: live2d_effect_from_preview(uniform),
                        target_drawable_ids: target_art_mesh_ids,
                    },
                );
            } else if has_session {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        self.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }
}

fn live2d_effect_from_preview(uniform: PreviewUniform) -> [f32; 4] {
    let strength = uniform.params0[0].clamp(0.0, 1.0);
    let brightness = uniform.params0[1].clamp(0.0, 2.0);
    let opacity = uniform.params3[1].clamp(0.0, 1.0);
    [
        (1.0 * (1.0 - strength) + uniform.tint_a[0] * strength * brightness).clamp(0.0, 2.0),
        (1.0 * (1.0 - strength) + uniform.tint_a[1] * strength * brightness).clamp(0.0, 2.0),
        (1.0 * (1.0 - strength) + uniform.tint_a[2] * strength * brightness).clamp(0.0, 2.0),
        opacity,
    ]
}
