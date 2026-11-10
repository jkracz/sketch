//! GPU resources for layer effects: the box shadow shader, the blur and composite passes,
//! the uniform slots every pass reads its parameters from, and a pool of offscreen textures.

use super::gpu::MSAA;
use super::mesh::Vertex;
use bytemuck::{Pod, Zeroable};
use std::num::NonZeroU64;

/// Blur with a Gaussian at most this many texels wide. A wider blur runs at a lower
/// resolution, where it is this wide again, so its cost stays flat as the radius grows.
pub const MAX_TEXEL_SIGMA: f64 = 6.0;
/// Largest edge of a blur texture. Anything bigger runs at a lower resolution.
pub const MAX_BLUR_EDGE: f64 = 2048.0;
/// Below this many pixels a blur changes nothing you can see.
pub const MIN_SIGMA_PX: f64 = 0.3;

/// The canvas transform of one render target. Matches `Globals` in every shader.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Globals {
    pub size_px: [f32; 2],
    pub scale: f32,
    pub zoom: f32,
    pub pan: [f32; 2],
    pub offset: [f32; 2],
}

/// Parameters of one composite or blur pass. Matches `Fx` in [`FX_SHADER`].
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
pub struct FxParams {
    /// Composite: target px of the texture's origin, then uv per px.
    /// Blur: uv origin and size of the source region.
    pub rect: [f32; 4],
    /// The uv range that holds valid texels (min, max).
    pub clamp: [f32; 4],
    /// Composite: premultiplied color. Blur: uv step per tap, sigma in taps, tap radius.
    pub color: [f32; 4],
    /// Composite without a mask: the target px rect to cover.
    pub quad: [f32; 4],
    /// Composite: mode, target width, target height. Blur: _, 1/width, 1/height of the output.
    pub params: [f32; 4],
}

pub const MODE_CONTENT: f32 = 0.0;
pub const MODE_TINT: f32 = 1.0;
pub const MODE_INSIDE: f32 = 2.0;

/// One box shadow. Matches the vertex inputs of [`SHADOW_SHADER`]. World units.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct ShadowInstance {
    /// Center, then half size.
    pub caster: [f32; 4],
    pub caster_radii: [f32; 4],
    pub layer: [f32; 4],
    pub layer_radii: [f32; 4],
    /// Straight alpha.
    pub color: [f32; 4],
    /// Sigma, then 1 for an inner shadow.
    pub params: [f32; 4],
}

const GLOBALS_WGSL: &str = r#"
struct Globals {
    size_px: vec2<f32>,
    scale: f32,
    zoom: f32,
    pan: vec2<f32>,
    offset: vec2<f32>,
};
@group(0) @binding(0) var<uniform> g: Globals;

fn to_clip(pos: vec2<f32>) -> vec4<f32> {
    let px = ((pos - g.pan) * g.zoom + g.offset) * g.scale;
    return vec4<f32>(px.x / g.size_px.x * 2.0 - 1.0, 1.0 - px.y / g.size_px.y * 2.0, 0.0, 1.0);
}
"#;

// The Gaussian shadow of a rounded box in closed form along x and sampled along y, after
// Evan Wallace's "Fast Rounded Rectangle Shadows" (CC0),
// https://madebyevan.com/shaders/fast-rounded-rectangle-shadows/. Changed here: a radius per
// corner, more samples, and the y integral normalized so a square box comes out exact.
const SHADOW_WGSL: &str = r#"
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    // Position relative to the caster's center and to the layer's.
    @location(0) pc: vec2<f32>,
    @location(1) pl: vec2<f32>,
    // Caster half size, sigma, inset.
    @location(2) @interpolate(flat) caster: vec4<f32>,
    @location(3) @interpolate(flat) caster_radii: vec4<f32>,
    @location(4) @interpolate(flat) layer_half: vec2<f32>,
    @location(5) @interpolate(flat) layer_radii: vec4<f32>,
    @location(6) @interpolate(flat) color: vec4<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32, @location(0) caster: vec4<f32>,
      @location(1) caster_radii: vec4<f32>, @location(2) layer: vec4<f32>,
      @location(3) layer_radii: vec4<f32>, @location(4) color: vec4<f32>,
      @location(5) params: vec4<f32>) -> VsOut {
    let px = 1.0 / (g.zoom * g.scale);
    // A hard shadow still gets an anti-aliased edge.
    let sigma = max(params.x, 0.4 * px);
    var center = caster.xy;
    var half = caster.zw + 3.0 * sigma + px;
    if (params.y > 0.5) {
        center = layer.xy;
        half = layer.zw + px;
    }
    let corner = (vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u)) * 2.0 - 1.0) * half;
    var o: VsOut;
    o.pos = to_clip(center + corner);
    o.pc = (center - caster.xy) + corner;
    o.pl = (center - layer.xy) + corner;
    o.caster = vec4<f32>(caster.zw, sigma, params.y);
    o.caster_radii = caster_radii;
    o.layer_half = layer.zw;
    o.layer_radii = layer_radii;
    o.color = color;
    return o;
}

fn erf2(x: vec2<f32>) -> vec2<f32> {
    let s = sign(x);
    let a = abs(x);
    var r = 1.0 + (0.278393 + (0.230389 + 0.078108 * (a * a)) * a) * a;
    r = r * r;
    return s - s / (r * r);
}

// Left and right edge of the box on row `y`. Radii are tl, tr, br, bl and y points down.
fn row_span(y: f32, half: vec2<f32>, r: vec4<f32>) -> vec2<f32> {
    let top = y < 0.0;
    let rl = select(r.w, r.x, top);
    let rr = select(r.z, r.y, top);
    let dl = max(abs(y) - (half.y - rl), 0.0);
    let dr = max(abs(y) - (half.y - rr), 0.0);
    return vec2<f32>(
        -half.x + rl - sqrt(max(rl * rl - dl * dl, 0.0)),
        half.x - rr + sqrt(max(rr * rr - dr * dr, 0.0)),
    );
}

fn box_shadow(p: vec2<f32>, half: vec2<f32>, r: vec4<f32>, sigma: f32) -> f32 {
    let inv = 0.70710678 / sigma;
    let lo = clamp(p.y - 3.0 * sigma, -half.y, half.y);
    let hi = clamp(p.y + 3.0 * sigma, -half.y, half.y);
    let step = (hi - lo) / 8.0;
    var acc = 0.0;
    var norm = 0.0;
    for (var i = 0; i < 8; i++) {
        let y = lo + step * (f32(i) + 0.5);
        let e = erf2((vec2<f32>(p.x) - row_span(y, half, r)) * inv);
        let dy = (p.y - y) / sigma;
        let w = exp(-0.5 * dy * dy);
        acc += 0.5 * (e.x - e.y) * w;
        norm += w;
    }
    let rows = erf2((vec2<f32>(hi, lo) - p.y) * inv);
    return acc / max(norm, 1e-6) * 0.5 * (rows.x - rows.y);
}

fn box_distance(p: vec2<f32>, half: vec2<f32>, r: vec4<f32>) -> f32 {
    let top = p.y < 0.0;
    let rr = select(select(r.z, r.y, top), select(r.w, r.x, top), p.x < 0.0);
    let q = abs(p) - half + rr;
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0))) - rr;
}

@fragment
fn fs(i: VsOut) -> @location(0) vec4<f32> {
    let k = g.zoom * g.scale;
    let shadow = box_shadow(i.pc, i.caster.xy, i.caster_radii, i.caster.z);
    let d = box_distance(i.pl, i.layer_half, i.layer_radii) * k;
    var a = shadow * (1.0 - clamp(-d, 0.0, 1.0));
    if (i.caster.w > 0.5) {
        a = (1.0 - shadow) * clamp(0.5 - d, 0.0, 1.0);
    }
    a *= i.color.a;
    return vec4<f32>(i.color.rgb * a, a);
}
"#;

const FX_WGSL: &str = r#"
struct Fx {
    rect: vec4<f32>,
    clamp: vec4<f32>,
    color: vec4<f32>,
    quad: vec4<f32>,
    params: vec4<f32>,
};
@group(1) @binding(0) var<uniform> fx: Fx;
@group(2) @binding(0) var tex: texture_2d<f32>;
@group(2) @binding(1) var samp: sampler;

fn corner(vi: u32) -> vec2<f32> {
    return vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));
}

// The texture painted back over the px rect `fx.quad`.
@vertex
fn vs_quad(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let px = fx.quad.xy + corner(vi) * fx.quad.zw;
    return vec4<f32>(px.x / fx.params.y * 2.0 - 1.0, 1.0 - px.y / fx.params.z * 2.0, 0.0, 1.0);
}

// The texture painted back through world geometry.
@vertex
fn vs_mask(@location(0) pos: vec2<f32>) -> @builtin(position) vec4<f32> {
    return to_clip(pos);
}

@fragment
fn fs_paint(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = clamp((p.xy - fx.rect.xy) * fx.rect.zw, fx.clamp.xy, fx.clamp.zw);
    let t = textureSampleLevel(tex, samp, uv, 0.0);
    if (fx.params.x < 0.5) {
        return t;
    }
    if (fx.params.x < 1.5) {
        return fx.color * t.a;
    }
    return fx.color * (1.0 - t.a);
}

@vertex
fn vs_full(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let c = corner(vi) * 2.0 - 1.0;
    return vec4<f32>(c.x, c.y, 0.0, 1.0);
}

// One direction of a Gaussian blur, or with radius 0 a resampling copy.
@fragment
fn fs_blur(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = fx.rect.xy + p.xy * fx.params.yz * fx.rect.zw;
    let radius = i32(fx.color.w);
    let sigma = max(fx.color.z, 1e-3);
    var acc = vec4<f32>(0.0);
    var norm = 0.0;
    for (var i = -radius; i <= radius; i++) {
        let d = f32(i) / sigma;
        let w = exp(-0.5 * d * d);
        let at = clamp(uv + fx.color.xy * f32(i), fx.clamp.xy, fx.clamp.zw);
        acc += textureSampleLevel(tex, samp, at, 0.0) * w;
        norm += w;
    }
    return acc / norm;
}
"#;

pub fn shadow_shader() -> String {
    format!("{GLOBALS_WGSL}{SHADOW_WGSL}")
}

pub fn fx_shader() -> String {
    format!("{GLOBALS_WGSL}{FX_WGSL}")
}

fn uniform_layout(device: &wgpu::Device, label: &str, size: usize) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: NonZeroU64::new(size as u64),
            },
            count: None,
        }],
    })
}

/// Uniform data for every pass of a frame, in one buffer. Each pass binds its slot by offset,
/// so a frame with many effects still uploads once.
pub struct Slots {
    pub globals_layout: wgpu::BindGroupLayout,
    pub fx_layout: wgpu::BindGroupLayout,
    buf: wgpu::Buffer,
    pub globals: wgpu::BindGroup,
    pub fx: wgpu::BindGroup,
    stride: u32,
    cap: u32,
    data: Vec<u8>,
}

impl Slots {
    pub fn new(device: &wgpu::Device) -> Slots {
        let globals_layout =
            uniform_layout(device, "canvas globals", std::mem::size_of::<Globals>());
        let fx_layout = uniform_layout(device, "fx params", std::mem::size_of::<FxParams>());
        let align = device.limits().min_uniform_buffer_offset_alignment.max(1);
        let stride = (std::mem::size_of::<FxParams>() as u32).div_ceil(align) * align;
        let cap = 16;
        let (buf, globals, fx) = Self::alloc(device, &globals_layout, &fx_layout, stride, cap);
        Slots {
            globals_layout,
            fx_layout,
            buf,
            globals,
            fx,
            stride,
            cap,
            data: Vec::new(),
        }
    }

    fn alloc(
        device: &wgpu::Device,
        globals_layout: &wgpu::BindGroupLayout,
        fx_layout: &wgpu::BindGroupLayout,
        stride: u32,
        cap: u32,
    ) -> (wgpu::Buffer, wgpu::BindGroup, wgpu::BindGroup) {
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pass uniforms"),
            size: (stride * cap) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = |layout, size: usize| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("pass uniforms"),
                layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &buf,
                        offset: 0,
                        size: NonZeroU64::new(size as u64),
                    }),
                }],
            })
        };
        let globals = bind(globals_layout, std::mem::size_of::<Globals>());
        let fx = bind(fx_layout, std::mem::size_of::<FxParams>());
        (buf, globals, fx)
    }

    /// Start a frame that needs at most `slots`. Passes are recorded against the buffer, so
    /// it has to be big enough before the first one.
    pub fn begin(&mut self, device: &wgpu::Device, slots: u32) {
        self.data.clear();
        if slots > self.cap {
            self.cap = slots.next_power_of_two();
            (self.buf, self.globals, self.fx) = Self::alloc(
                device,
                &self.globals_layout,
                &self.fx_layout,
                self.stride,
                self.cap,
            );
        }
    }

    /// Store `v` for this frame and return the offset to bind it at.
    pub fn push<T: Pod>(&mut self, v: &T) -> u32 {
        let at = self.data.len() as u32;
        if at / self.stride >= self.cap {
            debug_assert!(false, "more passes than `begin` was told about");
            return at.saturating_sub(self.stride);
        }
        self.data.extend_from_slice(bytemuck::bytes_of(v));
        self.data.resize((at + self.stride) as usize, 0);
        at
    }

    /// Upload the frame's slots. Call before submitting its passes.
    pub fn flush(&mut self, queue: &wgpu::Queue) {
        if !self.data.is_empty() {
            queue.write_buffer(&self.buf, 0, &self.data);
        }
    }
}

/// An offscreen texture. Sizes are rounded up so a blur that grows a little reuses it; a
/// pass draws into the top-left `size` it asked for.
pub struct PoolTexture {
    pub view: wgpu::TextureView,
    /// For sampling it. None for multisampled textures, which are only drawn into.
    pub bind: Option<wgpu::BindGroup>,
    pub alloc: [u32; 2],
    samples: u32,
    busy: bool,
    last_used: u64,
}

pub struct Pool {
    textures: Vec<PoolTexture>,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    format: wgpu::TextureFormat,
    frame: u64,
}

const POOL_STEP: u32 = 64;
/// Frames a texture may sit unused before it is freed.
const POOL_KEEP: u64 = 120;

impl Pool {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Pool {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fx texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("fx"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Pool {
            textures: Vec::new(),
            layout,
            sampler,
            format,
            frame: 0,
        }
    }

    pub fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    /// A texture view and a bind group that samples it.
    pub fn texture(
        &self,
        device: &wgpu::Device,
        size: [u32; 2],
        samples: u32,
    ) -> (wgpu::TextureView, Option<wgpu::BindGroup>) {
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("fx"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: if samples > 1 {
                wgpu::TextureUsages::RENDER_ATTACHMENT
            } else {
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING
            },
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        let bind = (samples == 1).then(|| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fx texture"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            })
        });
        (view, bind)
    }

    /// A free texture that holds `size`, until [`release`](Pool::release).
    pub fn acquire(&mut self, device: &wgpu::Device, size: [u32; 2], samples: u32) -> usize {
        let alloc = size.map(|s| s.max(1).div_ceil(POOL_STEP) * POOL_STEP);
        let found = (self.textures.iter())
            .position(|t| !t.busy && t.alloc == alloc && t.samples == samples);
        let i = found.unwrap_or_else(|| {
            let (view, bind) = self.texture(device, alloc, samples);
            self.textures.push(PoolTexture {
                view,
                bind,
                alloc,
                samples,
                busy: false,
                last_used: 0,
            });
            self.textures.len() - 1
        });
        self.textures[i].busy = true;
        self.textures[i].last_used = self.frame;
        i
    }

    /// Passes recorded from now on may draw over it.
    pub fn release(&mut self, i: usize) {
        self.textures[i].busy = false;
    }

    pub fn get(&self, i: usize) -> &PoolTexture {
        &self.textures[i]
    }

    /// Free what the frame held and drop textures no frame has wanted for a while.
    pub fn end_frame(&mut self) {
        self.frame += 1;
        let frame = self.frame;
        for t in &mut self.textures {
            t.busy = false;
        }
        self.textures.retain(|t| frame - t.last_used <= POOL_KEEP);
    }
}

/// The pipelines effects draw with.
pub struct FxPipelines {
    pub shadow: wgpu::RenderPipeline,
    /// A blurred texture painted over a px rect.
    pub quad: wgpu::RenderPipeline,
    /// The same through world geometry: blended over, or replacing what is there.
    pub mask: [wgpu::RenderPipeline; 2],
    pub blur: wgpu::RenderPipeline,
}

/// Premultiplied source over.
pub const OVER: wgpu::BlendState = wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING;

impl FxPipelines {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        slots: &Slots,
        pool: &Pool,
    ) -> FxPipelines {
        let module = |label, source: String| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            })
        };
        let shadow_module = module("box shadow", shadow_shader());
        let fx_module = module("fx", fx_shader());
        let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("box shadow"),
            bind_group_layouts: &[Some(&slots.globals_layout)],
            immediate_size: 0,
        });
        let fx_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fx"),
            bind_group_layouts: &[
                Some(&slots.globals_layout),
                Some(&slots.fx_layout),
                Some(pool.layout()),
            ],
            immediate_size: 0,
        });
        let strip = wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        };
        let multisample = |count| wgpu::MultisampleState {
            count,
            mask: !0,
            alpha_to_coverage_enabled: false,
        };
        let shadow_attrs = wgpu::vertex_attr_array![
            0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4,
            5 => Float32x4
        ];
        let shadow = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("box shadow"),
            layout: Some(&shadow_layout),
            vertex: wgpu::VertexState {
                module: &shadow_module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<ShadowInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &shadow_attrs,
                })],
            },
            primitive: strip,
            depth_stencil: None,
            multisample: multisample(MSAA),
            fragment: Some(wgpu::FragmentState {
                module: &shadow_module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(OVER),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let vertex_attrs = wgpu::vertex_attr_array![0 => Float32x2];
        let fx = |label,
                  vs: &str,
                  fs: &str,
                  world: bool,
                  primitive: wgpu::PrimitiveState,
                  samples: u32,
                  blend: Option<wgpu::BlendState>| {
            let world_buffer = [Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &vertex_attrs,
            })];
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&fx_layout),
                vertex: wgpu::VertexState {
                    module: &fx_module,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: if world { &world_buffer } else { &[] },
                },
                primitive,
                depth_stencil: None,
                multisample: multisample(samples),
                fragment: Some(wgpu::FragmentState {
                    module: &fx_module,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let list = wgpu::PrimitiveState::default();
        FxPipelines {
            shadow,
            quad: fx(
                "fx quad",
                "vs_quad",
                "fs_paint",
                false,
                strip,
                MSAA,
                Some(OVER),
            ),
            mask: [
                fx(
                    "fx mask",
                    "vs_mask",
                    "fs_paint",
                    true,
                    list,
                    MSAA,
                    Some(OVER),
                ),
                fx("fx replace", "vs_mask", "fs_paint", true, list, MSAA, None),
            ],
            blur: fx("fx blur", "vs_full", "fs_blur", false, strip, 1, None),
        }
    }
}
