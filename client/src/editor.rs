//! wgpu 3D ship-editor viewport.
//!
//! Renders the active draft's blocks (instanced unit cubes) + a floor + a
//! translucent ghost cube to an offscreen texture, which the Designer screen
//! shows as an egui image. Provides an orbit camera and cursor raycast picking.

use eframe::egui;
use egui_wgpu::RenderState;
use glam::{Mat4, Vec3, Vec4Swizzles};
use wgpu::util::DeviceExt;

use crate::module_bindings::BlockType;

const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

// ── GPU data ───────────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    pos: [f32; 3],
    normal: [f32; 3],
}

/// Per-cube instance: a position offset, a scale, and an RGBA color.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Instance {
    pub offset: [f32; 3],
    pub scale: [f32; 3],
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
}

const SHADER: &str = r#"
struct Camera { view_proj: mat4x4<f32> };
@group(0) @binding(0) var<uniform> camera: Camera;

struct VsIn {
  @location(0) pos: vec3<f32>,
  @location(1) normal: vec3<f32>,
  @location(2) offset: vec3<f32>,
  @location(3) scale: vec3<f32>,
  @location(4) color: vec4<f32>,
};
struct VsOut {
  @builtin(position) clip: vec4<f32>,
  @location(0) color: vec4<f32>,
  @location(1) normal: vec3<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
  var out: VsOut;
  let world = in.pos * in.scale + in.offset;
  out.clip = camera.view_proj * vec4<f32>(world, 1.0);
  out.color = in.color;
  out.normal = in.normal;
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  let light = normalize(vec3<f32>(0.4, 1.0, 0.6));
  let d = max(dot(normalize(in.normal), light), 0.0);
  let shade = 0.35 + 0.65 * d;
  return vec4<f32>(in.color.rgb * shade, in.color.a);
}
"#;

fn cube_mesh() -> (Vec<Vertex>, Vec<u16>) {
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        ([1.0, 0.0, 0.0], [[0.5, -0.5, -0.5], [0.5, 0.5, -0.5], [0.5, 0.5, 0.5], [0.5, -0.5, 0.5]]),
        ([-1.0, 0.0, 0.0], [[-0.5, -0.5, 0.5], [-0.5, 0.5, 0.5], [-0.5, 0.5, -0.5], [-0.5, -0.5, -0.5]]),
        ([0.0, 1.0, 0.0], [[-0.5, 0.5, -0.5], [-0.5, 0.5, 0.5], [0.5, 0.5, 0.5], [0.5, 0.5, -0.5]]),
        ([0.0, -1.0, 0.0], [[-0.5, -0.5, 0.5], [-0.5, -0.5, -0.5], [0.5, -0.5, -0.5], [0.5, -0.5, 0.5]]),
        ([0.0, 0.0, 1.0], [[0.5, -0.5, 0.5], [0.5, 0.5, 0.5], [-0.5, 0.5, 0.5], [-0.5, -0.5, 0.5]]),
        ([0.0, 0.0, -1.0], [[-0.5, -0.5, -0.5], [-0.5, 0.5, -0.5], [0.5, 0.5, -0.5], [0.5, -0.5, -0.5]]),
    ];
    let mut verts = Vec::new();
    let mut idx: Vec<u16> = Vec::new();
    for (normal, corners) in faces {
        let base = verts.len() as u16;
        for pos in corners {
            verts.push(Vertex { pos, normal });
        }
        idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (verts, idx)
}

// ── Orbit camera ─────────────────────────────────────────────────────────────

pub struct OrbitCamera {
    pub yaw: f32,
    pub pitch: f32,
    pub radius: f32,
    pub target: Vec3,
}

impl OrbitCamera {
    pub fn new() -> Self {
        Self { yaw: 0.8, pitch: 0.5, radius: 12.0, target: Vec3::ZERO }
    }

    fn eye(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        self.target + Vec3::new(self.radius * cp * sy, self.radius * sp, self.radius * cp * cy)
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let view = Mat4::look_at_rh(self.eye(), self.target, Vec3::Y);
        let proj = Mat4::perspective_rh(60f32.to_radians(), aspect.max(0.01), 0.1, 500.0);
        proj * view
    }

    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * 0.01;
        self.pitch = (self.pitch + dy * 0.01).clamp(-1.4, 1.4);
    }

    pub fn zoom(&mut self, amount: f32) {
        self.radius = (self.radius * (1.0 - amount * 0.1)).clamp(3.0, 80.0);
    }
}

// ── Viewport (offscreen render target + pipeline) ────────────────────────────

pub struct Viewport {
    pipeline: wgpu::RenderPipeline,
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    index_count: u32,
    camera_buf: wgpu::Buffer,
    camera_bg: wgpu::BindGroup,
    color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    tex_id: egui::TextureId,
    size: (u32, u32),
}

fn make_targets(device: &wgpu::Device, size: (u32, u32)) -> (wgpu::TextureView, wgpu::TextureView) {
    let extent = wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 };
    let color = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("editor color"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: COLOR_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("editor depth"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    (color.create_view(&Default::default()), depth.create_view(&Default::default()))
}

impl Viewport {
    pub fn new(rs: &RenderState) -> Self {
        let device = &rs.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("editor shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let camera_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let camera_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera buf"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera bg"),
            layout: &camera_bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: camera_buf.as_entire_binding() }],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("editor layout"),
            bind_group_layouts: &[Some(&camera_bgl)],
            immediate_size: 0,
        });

        let vbo_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
        let inst_attrs = wgpu::vertex_attr_array![2 => Float32x3, 3 => Float32x3, 4 => Float32x4];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("editor pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &vbo_attrs,
                    },
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Instance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &inst_attrs,
                    },
                ],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COLOR_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let (verts, idx) = cube_mesh();
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cube vbuf"),
            contents: bytemuck::cast_slice(&verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cube ibuf"),
            contents: bytemuck::cast_slice(&idx),
            usage: wgpu::BufferUsages::INDEX,
        });

        let size = (16, 16);
        let (color_view, depth_view) = make_targets(device, size);
        let tex_id = rs.renderer.write().register_native_texture(
            device,
            &color_view,
            wgpu::FilterMode::Linear,
        );

        Self {
            pipeline,
            vbuf,
            ibuf,
            index_count: idx.len() as u32,
            camera_buf,
            camera_bg,
            color_view,
            depth_view,
            tex_id,
            size,
        }
    }

    fn ensure_size(&mut self, rs: &RenderState, size: (u32, u32)) {
        let size = (size.0.max(1), size.1.max(1));
        if size == self.size {
            return;
        }
        self.size = size;
        let (color_view, depth_view) = make_targets(&rs.device, size);
        self.color_view = color_view;
        self.depth_view = depth_view;
        rs.renderer.write().update_egui_texture_from_wgpu_texture(
            &rs.device,
            &self.color_view,
            wgpu::FilterMode::Linear,
            self.tex_id,
        );
    }

    /// Render the scene to the offscreen texture and return its egui id.
    pub fn render(
        &mut self,
        rs: &RenderState,
        size: (u32, u32),
        instances: &[Instance],
        view_proj: Mat4,
    ) -> egui::TextureId {
        self.ensure_size(rs, size);

        let camera = CameraUniform { view_proj: view_proj.to_cols_array_2d() };
        rs.queue.write_buffer(&self.camera_buf, 0, bytemuck::bytes_of(&camera));

        let inst_buf = rs.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("editor instances"),
            contents: bytemuck::cast_slice(instances),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let mut encoder = rs
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("editor encoder") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("editor pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.color_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.02, g: 0.03, b: 0.06, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if !instances.is_empty() {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.camera_bg, &[]);
                pass.set_vertex_buffer(0, self.vbuf.slice(..));
                pass.set_vertex_buffer(1, inst_buf.slice(..));
                pass.set_index_buffer(self.ibuf.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..self.index_count, 0, 0..instances.len() as u32);
            }
        }
        rs.queue.submit(std::iter::once(encoder.finish()));
        self.tex_id
    }
}

// ── Colors + shared conversion ───────────────────────────────────────────────

pub fn block_color(t: &BlockType) -> [f32; 4] {
    match t {
        BlockType::Hull => [0.55, 0.57, 0.60, 1.0],
        BlockType::Engine => [0.95, 0.55, 0.20, 1.0],
        BlockType::Weapon => [0.85, 0.25, 0.25, 1.0],
        BlockType::Reactor => [0.30, 0.80, 0.45, 1.0],
        BlockType::Sensor => [0.30, 0.65, 0.95, 1.0],
        BlockType::CommandCore => [0.88, 0.80, 0.32, 1.0],
    }
}

pub fn to_shared(t: &BlockType) -> starframe_shared::BlockType {
    use starframe_shared::BlockType as S;
    match t {
        BlockType::Hull => S::Hull,
        BlockType::Engine => S::Engine,
        BlockType::Weapon => S::Weapon,
        BlockType::Reactor => S::Reactor,
        BlockType::Sensor => S::Sensor,
        BlockType::CommandCore => S::CommandCore,
    }
}

// ── Raycast picking ──────────────────────────────────────────────────────────

pub struct Pick {
    /// Empty cell where a new block would go (the hovered face's neighbour, or
    /// the ground cell when pointing at empty space).
    pub ghost: [i32; 3],
    /// The existing block cell under the cursor, if any (target for removal).
    pub remove: Option<[i32; 3]>,
}

/// Cast a ray through `uv` (0..1 over the viewport) and resolve the place/remove
/// cells against the existing `cells` and the ground plane.
pub fn pick(view_proj: Mat4, uv: (f32, f32), cells: &[[i32; 3]]) -> Option<Pick> {
    let ndc = glam::vec2(uv.0 * 2.0 - 1.0, 1.0 - uv.1 * 2.0);
    let inv = view_proj.inverse();
    let unproject = |z: f32| {
        let p = inv * glam::vec4(ndc.x, ndc.y, z, 1.0);
        p.xyz() / p.w
    };
    let near = unproject(0.0);
    let far = unproject(1.0);
    let dir = (far - near).normalize_or_zero();
    if dir == Vec3::ZERO {
        return None;
    }
    let origin = near;

    let mut best: Option<(f32, [i32; 3], Vec3)> = None;
    for c in cells {
        let center = Vec3::new(c[0] as f32, c[1] as f32, c[2] as f32);
        if let Some((t, normal)) = ray_aabb(origin, dir, center - Vec3::splat(0.5), center + Vec3::splat(0.5)) {
            if t >= 0.0 && best.map_or(true, |(bt, _, _)| t < bt) {
                best = Some((t, *c, normal));
            }
        }
    }
    if let Some((_, cell, n)) = best {
        let ghost = [cell[0] + n.x as i32, cell[1] + n.y as i32, cell[2] + n.z as i32];
        return Some(Pick { ghost, remove: Some(cell) });
    }

    // Ground plane at y = -0.5 (the underside of the y = 0 layer).
    if dir.y.abs() > 1e-5 {
        let t = (-0.5 - origin.y) / dir.y;
        if t > 0.0 {
            let h = origin + dir * t;
            return Some(Pick {
                ghost: [h.x.round() as i32, 0, h.z.round() as i32],
                remove: None,
            });
        }
    }
    None
}

fn ray_aabb(o: Vec3, d: Vec3, min: Vec3, max: Vec3) -> Option<(f32, Vec3)> {
    let inv = Vec3::new(1.0 / d.x, 1.0 / d.y, 1.0 / d.z);
    let t0 = (min - o) * inv;
    let t1 = (max - o) * inv;
    let tsmall = t0.min(t1);
    let tbig = t0.max(t1);
    let tmin = tsmall.max_element();
    let tmax = tbig.min_element();
    if tmax < tmin.max(0.0) {
        return None;
    }
    let normal = if tsmall.x >= tsmall.y && tsmall.x >= tsmall.z {
        Vec3::new(-d.x.signum(), 0.0, 0.0)
    } else if tsmall.y >= tsmall.z {
        Vec3::new(0.0, -d.y.signum(), 0.0)
    } else {
        Vec3::new(0.0, 0.0, -d.z.signum())
    };
    Some((tmin, normal))
}
