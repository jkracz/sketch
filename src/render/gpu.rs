//! wgpu pipelines for the canvas, plus offscreen rendering for screenshots and PNG export.
//!
//! A frame is mostly one multisampled pass over cached world geometry. Effects add to it.
//! The shader draws box shadows in that same pass. A blur draws its content into a texture
//! first, blurs it in two passes and paints the result back. A background blur ends the pass,
//! blurs a copy of what is painted so far, and carries on in a new pass.

use super::fx::{
    FxParams, FxPipelines, Globals, MAX_BLUR_EDGE, MAX_TEXEL_SIGMA, MIN_SIGMA_PX, MODE_CONTENT,
    MODE_INSIDE, MODE_TINT, Pool, ShadowInstance, Slots,
};
use super::mesh::Vertex;
use super::scene::{BlurPaint, CanvasFrame, Op};
use super::tess::{self, Piece};
use crate::assets;
use crate::geom::{Rect, V2, v2};
use crate::model::BlendMode;
use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use wgpu::util::DeviceExt;

pub const MSAA: u32 = 4;

/// Colors are authored in sRGB and blended in gamma space (like egui), so targets are non-sRGB.
pub const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

const SHADER: &str = r#"
struct Globals {
    size_px: vec2<f32>,
    scale: f32,
    zoom: f32,
    pan: vec2<f32>,
    offset: vec2<f32>,
};
@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var ramp_tex: texture_2d<f32>;
@group(1) @binding(1) var ramp_samp: sampler;
@group(2) @binding(0) var img_tex: texture_2d<f32>;
@group(2) @binding(1) var img_samp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) kind: u32,
};

@vertex
fn vs(@location(0) pos: vec2<f32>, @location(1) color: vec4<f32>, @location(2) uv: vec2<f32>,
      @location(3) kind: u32) -> VsOut {
    let logical = (pos - g.pan) * g.zoom + g.offset;
    let px = logical * g.scale;
    var o: VsOut;
    o.pos = vec4<f32>(px.x / g.size_px.x * 2.0 - 1.0, 1.0 - px.y / g.size_px.y * 2.0, 0.0, 1.0);
    o.color = color;
    o.uv = uv;
    o.kind = kind;
    return o;
}

fn ramp(t: f32, row: u32) -> vec4<f32> {
    let v = (f32(row) + 0.5) / 256.0;
    return textureSampleLevel(ramp_tex, ramp_samp, vec2<f32>(clamp(t, 0.0, 1.0), v), 0.0);
}

@fragment
fn fs(i: VsOut) -> @location(0) vec4<f32> {
    let k = i.kind & 255u;
    let row = i.kind >> 8u;
    var c = i.color;
    if (k == 1u) {
        let r = ramp(i.uv.x, row);
        c = vec4<f32>(r.rgb, r.a * i.color.a);
    } else if (k == 2u) {
        let r = ramp(length(i.uv), row);
        c = vec4<f32>(r.rgb, r.a * i.color.a);
    } else if (k >= 3u) {
        if (k == 3u && (i.uv.x < 0.0 || i.uv.y < 0.0 || i.uv.x > 1.0 || i.uv.y > 1.0)) {
            discard;
        }
        let t = textureSample(img_tex, img_samp, i.uv);
        c = vec4<f32>(t.rgb, t.a * i.color.a);
    }
    // Premultiplied output; every pipeline blends premultiplied.
    return vec4<f32>(c.rgb * c.a, c.a);
}
"#;

/// GPU pipelines exist for the blend modes fixed-function blending can express.
fn blend_key(b: BlendMode) -> usize {
    match b {
        BlendMode::Multiply => 1,
        BlendMode::Screen => 2,
        BlendMode::Darken => 3,
        BlendMode::Lighten => 4,
        _ => 0,
    }
}

fn blend_state(key: usize) -> wgpu::BlendState {
    use wgpu::{BlendComponent as C, BlendFactor as F, BlendOperation as O};
    let alpha = C {
        src_factor: F::One,
        dst_factor: F::OneMinusSrcAlpha,
        operation: O::Add,
    };
    let color = match key {
        1 => C {
            src_factor: F::Dst,
            dst_factor: F::OneMinusSrcAlpha,
            operation: O::Add,
        },
        2 => C {
            src_factor: F::One,
            dst_factor: F::OneMinusSrc,
            operation: O::Add,
        },
        3 => C {
            src_factor: F::One,
            dst_factor: F::One,
            operation: O::Min,
        },
        4 => C {
            src_factor: F::One,
            dst_factor: F::One,
            operation: O::Max,
        },
        _ => alpha,
    };
    wgpu::BlendState { color, alpha }
}

// ---------------------------------------------------------------------------------- world plan

struct Draw {
    range: Range<u32>,
    clip: Option<Rect>,
    blend: usize,
    image: Option<String>,
}

/// The ops of a frame, with geometry replaced by ranges of the uploaded buffers.
enum Step {
    Draw(Draw),
    Shadows {
        range: Range<u32>,
        clip: Option<Rect>,
    },
    /// Index into `World::blurs`.
    Blur(usize),
    /// Index into `World::backdrops`.
    Backdrop(usize),
}

struct BlurPlan {
    steps: Vec<Step>,
    bounds: Rect,
    sigma: f64,
    offset: V2,
    spread: f64,
    paint: PlanPaint,
    clip: Option<Rect>,
}

enum PlanPaint {
    Content,
    Tint([f32; 4]),
    Inside { color: [f32; 4], mask: Range<u32> },
}

struct BackdropPlan {
    mask: Range<u32>,
    bounds: Rect,
    sigma: f64,
    opacity: f64,
    clip: Option<Rect>,
}

/// Reuse world geometry across idle frames and camera pans.
struct World {
    /// Held so a new list can't reuse this one's address and pass for it.
    ops: Arc<Vec<Op>>,
    signature: Vec<u64>,
    vb: Option<wgpu::Buffer>,
    ib: Option<wgpu::Buffer>,
    shadows: Option<wgpu::Buffer>,
    steps: Vec<Step>,
    blurs: Vec<BlurPlan>,
    backdrops: Vec<BackdropPlan>,
}

fn rect_bits(r: &Rect) -> [u64; 4] {
    [r.x, r.y, r.w, r.h].map(f64::to_bits)
}

fn signature(ops: &[Op], out: &mut Vec<u64>) {
    let clip = |c: &Option<Rect>, out: &mut Vec<u64>| match c {
        Some(c) => {
            out.push(1);
            out.extend(rect_bits(c));
        }
        None => out.push(0),
    };
    let color = |c: &[f32; 4], out: &mut Vec<u64>| out.extend(c.map(|v| v.to_bits() as u64));
    let pieces = |p: &Arc<Vec<Piece>>| Arc::as_ptr(p) as usize as u64;
    for op in ops {
        match op {
            Op::Item(it) => {
                out.extend([0, pieces(&it.pieces), blend_key(it.blend) as u64]);
                clip(&it.clip, out);
            }
            Op::Shadow(s) => {
                out.extend([1, s.inset as u64, s.sigma.to_bits()]);
                for b in [&s.caster, &s.layer] {
                    out.extend(rect_bits(&b.rect));
                    out.extend(b.radii.map(f64::to_bits));
                }
                color(&s.color, out);
                clip(&s.clip, out);
            }
            Op::Blur(b) => {
                out.extend([2, b.sigma.to_bits(), b.offset.x.to_bits()]);
                out.extend([b.offset.y.to_bits(), b.spread.to_bits()]);
                out.extend(rect_bits(&b.bounds));
                match &b.paint {
                    BlurPaint::Content => out.push(0),
                    BlurPaint::Tint(c) => {
                        out.push(1);
                        color(c, out);
                    }
                    BlurPaint::Inside { color: c, mask } => {
                        out.extend([2, pieces(mask)]);
                        color(c, out);
                    }
                }
                clip(&b.clip, out);
                out.push(b.content.len() as u64);
                signature(&b.content, out);
            }
            Op::Backdrop(b) => {
                out.extend([3, pieces(&b.mask), b.sigma.to_bits()]);
                out.push(b.opacity.to_bits());
                out.extend(rect_bits(&b.bounds));
                clip(&b.clip, out);
            }
        }
    }
}

#[derive(Default)]
struct Builder {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    shadows: Vec<ShadowInstance>,
    blurs: Vec<BlurPlan>,
    backdrops: Vec<BackdropPlan>,
}

impl Builder {
    fn mesh(&mut self, piece: &Piece) -> Range<u32> {
        let base = self.vertices.len() as u32;
        let start = self.indices.len() as u32;
        self.vertices.extend_from_slice(&piece.mesh.vertices);
        self.indices
            .extend(piece.mesh.indices.iter().map(|i| i + base));
        start..self.indices.len() as u32
    }

    fn mask(&mut self, pieces: &[Piece]) -> Range<u32> {
        let start = self.indices.len() as u32;
        for p in pieces {
            self.mesh(p);
        }
        start..self.indices.len() as u32
    }

    fn steps(&mut self, ops: &[Op]) -> Vec<Step> {
        let mut out: Vec<Step> = Vec::new();
        for op in ops {
            match op {
                Op::Item(it) => {
                    let blend = blend_key(it.blend);
                    for p in it.pieces.iter() {
                        let range = self.mesh(p);
                        match out.last_mut() {
                            Some(Step::Draw(d))
                                if d.clip == it.clip
                                    && d.blend == blend
                                    && d.image == p.image
                                    && d.range.end == range.start =>
                            {
                                d.range.end = range.end
                            }
                            _ => out.push(Step::Draw(Draw {
                                range,
                                clip: it.clip,
                                blend,
                                image: p.image.clone(),
                            })),
                        }
                    }
                }
                Op::Shadow(s) => {
                    let at = self.shadows.len() as u32;
                    let rect = |r: &Rect| {
                        let c = r.center();
                        [c.x, c.y, r.w * 0.5, r.h * 0.5].map(|v| v as f32)
                    };
                    self.shadows.push(ShadowInstance {
                        caster: rect(&s.caster.rect),
                        caster_radii: s.caster.radii.map(|r| r as f32),
                        layer: rect(&s.layer.rect),
                        layer_radii: s.layer.radii.map(|r| r as f32),
                        color: s.color,
                        params: [s.sigma as f32, s.inset as u8 as f32, 0.0, 0.0],
                    });
                    match out.last_mut() {
                        Some(Step::Shadows { range, clip })
                            if *clip == s.clip && range.end == at =>
                        {
                            range.end = at + 1
                        }
                        _ => out.push(Step::Shadows {
                            range: at..at + 1,
                            clip: s.clip,
                        }),
                    }
                }
                Op::Blur(b) => {
                    let steps = self.steps(&b.content);
                    let paint = match &b.paint {
                        BlurPaint::Content => PlanPaint::Content,
                        BlurPaint::Tint(c) => PlanPaint::Tint(*c),
                        BlurPaint::Inside { color, mask } => PlanPaint::Inside {
                            color: *color,
                            mask: self.mask(mask),
                        },
                    };
                    self.blurs.push(BlurPlan {
                        steps,
                        bounds: b.bounds,
                        sigma: b.sigma,
                        offset: b.offset,
                        spread: b.spread,
                        paint,
                        clip: b.clip,
                    });
                    out.push(Step::Blur(self.blurs.len() - 1));
                }
                Op::Backdrop(b) => {
                    let mask = self.mask(&b.mask);
                    self.backdrops.push(BackdropPlan {
                        mask,
                        bounds: b.bounds,
                        sigma: b.sigma,
                        opacity: b.opacity,
                        clip: b.clip,
                    });
                    out.push(Step::Backdrop(self.backdrops.len() - 1));
                }
            }
        }
        out
    }
}

/// Maps world units to target pixels with `px = world * k + t`.
#[derive(Clone, Copy)]
struct Map {
    k: f64,
    t: V2,
}

impl Map {
    fn rect(&self, r: &Rect) -> Rect {
        Rect::new(
            r.x * self.k + self.t.x,
            r.y * self.k + self.t.y,
            r.w * self.k,
            r.h * self.k,
        )
    }

    /// Pan is subtracted before scaling, as on the canvas, so far-off content keeps its
    /// precision in `f32`.
    fn globals(&self, size: [u32; 2]) -> Globals {
        Globals {
            size_px: size.map(|s| s as f32),
            scale: 1.0,
            zoom: self.k as f32,
            pan: [(-self.t.x / self.k) as f32, (-self.t.y / self.k) as f32],
            offset: [0.0; 2],
        }
    }
}

struct Target {
    size: [u32; 2],
    map: Map,
    /// The px that end up on screen. Nothing is drawn outside.
    visible: Rect,
    globals: u32,
    /// The canvas itself. Only it can blur what is behind a layer.
    main: bool,
}

type Scissor = [u32; 4];

impl Target {
    /// `r`, in target px, cut to the visible part and to `clip`, which is in world units.
    fn shown(&self, r: Rect, clip: &Option<Rect>) -> Option<Rect> {
        let r = r.intersection(&self.visible)?;
        match clip {
            Some(c) => r.intersection(&self.map.rect(c)),
            None => Some(r),
        }
    }

    fn scissor(&self, r: &Rect) -> Option<Scissor> {
        let [w, h] = self.size.map(|s| s as f64);
        let x0 = r.x.floor().clamp(0.0, w);
        let y0 = r.y.floor().clamp(0.0, h);
        let x1 = r.right().ceil().clamp(0.0, w);
        let y1 = r.bottom().ceil().clamp(0.0, h);
        (x1 > x0 && y1 > y0).then_some([x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32])
    }

    fn clip(&self, clip: &Option<Rect>) -> Option<Scissor> {
        self.scissor(&self.shown(self.visible, clip)?)
    }
}

#[derive(Clone, Copy)]
enum Tex {
    Pool(usize),
    /// The copy of the canvas a background blur reads.
    Canvas,
}

/// One draw of a pass, resolved for its target.
enum Cmd {
    Draw {
        range: Range<u32>,
        scissor: Scissor,
        blend: usize,
        image: Option<String>,
        globals: u32,
        /// From the per-frame screen buffers that hold the chrome, not the world buffers.
        screen: bool,
    },
    Shadows {
        range: Range<u32>,
        scissor: Scissor,
        globals: u32,
    },
    Paint {
        tex: usize,
        fx: u32,
        scissor: Scissor,
        mask: Option<Range<u32>>,
        /// Mix with what is there by this much, in place of blending over it.
        mix: Option<f64>,
        globals: u32,
    },
    /// Blur what the pass has painted so far. Ends the pass.
    Backdrop {
        plan: usize,
        /// Visible bounds in target pixels.
        shown: Rect,
    },
}

enum BlurMode {
    Skip,
    /// Too slight to see, so its content is drawn as it is.
    Inline,
    Offscreen(Job),
}

struct Job {
    plan: usize,
    /// The target px that show it.
    shown: Rect,
    /// Target px of its texture's top-left.
    origin: V2,
    /// Target px per texel.
    down: f64,
    size: [u32; 2],
}

/// An atlas page, one texture that many blurs share so they also share their passes.
struct Page {
    size: [u32; 2],
    tex: usize,
    jobs: Vec<(usize, [u32; 2])>,
}

const PAGE_EDGE: u32 = 2048;

/// Most times a background blur halves the canvas. Sixteen take any canvas down to a pixel.
const MAX_HALVINGS: usize = 16;

#[derive(Default)]
struct Packed {
    size: [u32; 2],
    rects: Vec<(usize, [u32; 2])>,
}

fn pack(sizes: &[[u32; 2]]) -> Vec<Packed> {
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by_key(|i| std::cmp::Reverse(sizes[*i][1]));
    let area: f64 = sizes.iter().map(|s| s[0] as f64 * s[1] as f64).sum();
    let widest = sizes.iter().map(|s| s[0]).max().unwrap_or(1);
    // Roughly square, so a few small blurs don't ask for a whole page.
    let width = ((area * 1.2).sqrt().ceil() as u32).clamp(widest, PAGE_EDGE.max(widest));
    let mut pages: Vec<Packed> = Vec::new();
    let (mut x, mut y, mut shelf) = (0, 0, 0);
    for i in order {
        let [w, h] = sizes[i];
        if x + w > width {
            (x, y, shelf) = (0, y + shelf, 0);
        }
        if pages.is_empty() || y + h > PAGE_EDGE.max(h) {
            pages.push(Packed::default());
            (x, y, shelf) = (0, 0, 0);
        }
        let page = pages.last_mut().unwrap();
        page.rects.push((i, [x, y]));
        page.size = [page.size[0].max(x + w), page.size[1].max(y + h)];
        x += w;
        shelf = shelf.max(h);
    }
    pages
}

/// Source region in texture pixels.
struct Source {
    tex: Tex,
    at: [f64; 2],
    size: [f64; 2],
    alloc: [u32; 2],
}

impl Source {
    fn pool(tex: usize, at: [u32; 2], size: [u32; 2], alloc: [u32; 2]) -> Source {
        Source {
            tex: Tex::Pool(tex),
            at: at.map(|v| v as f64),
            size: size.map(|v| v as f64),
            alloc,
        }
    }

    fn uv(&self) -> [f32; 4] {
        let [w, h] = self.alloc.map(|a| a as f64);
        [
            self.at[0] / w,
            self.at[1] / h,
            self.size[0] / w,
            self.size[1] / h,
        ]
        .map(|v| v as f32)
    }

    /// The uv range of the texels inside the region, so taps past its edge repeat the edge
    /// instead of reading a neighbor or what another pass left there.
    fn valid_uv(&self) -> [f32; 4] {
        let [w, h] = self.alloc.map(|a| a as f64);
        let lo = [(self.at[0] + 0.5) / w, (self.at[1] + 0.5) / h];
        let hi = [
            (self.at[0] + self.size[0] - 0.5) / w,
            (self.at[1] + self.size[1] - 0.5) / h,
        ];
        [lo[0], lo[1], hi[0].max(lo[0]), hi[1].max(lo[1])].map(|v| v as f32)
    }
}

/// `dir` selects the Gaussian axis, 0 for x and 1 for y. None makes a resampling copy.
struct BlurPart {
    src: Source,
    at: [u32; 2],
    size: [u32; 2],
    /// In source texels.
    sigma: f64,
    dir: Option<usize>,
}

struct Attachments<'a> {
    view: &'a wgpu::TextureView,
    resolve: &'a wgpu::TextureView,
    load: wgpu::LoadOp<wgpu::Color>,
    store: wgpu::StoreOp,
}

pub struct CanvasRenderer {
    pipelines: Vec<wgpu::RenderPipeline>,
    fx: FxPipelines,
    slots: Slots,
    pool: Pool,
    ramp_tex: wgpu::Texture,
    ramp_bg: wgpu::BindGroup,
    ramp_version: u64,
    img_layout: wgpu::BindGroupLayout,
    img_sampler: wgpu::Sampler,
    blank_img: wgpu::BindGroup,
    images: HashMap<String, wgpu::BindGroup>,
    world: Option<World>,
    msaa: Option<(wgpu::TextureView, [u32; 2])>,
    /// Copy of the canvas for background blurs: view, sampling bind group, size.
    canvas_copy: Option<(wgpu::TextureView, wgpu::BindGroup, [u32; 2])>,
    /// Per-frame screen-space vertex and index buffers.
    screen: Option<(wgpu::Buffer, wgpu::Buffer)>,
    format: wgpu::TextureFormat,
    /// World geometry uploads (for tests: panning must not re-upload).
    pub uploads: u64,
    /// Render passes recorded by the last `render`, for tests and profiling.
    pub last_passes: u32,
}

fn texture_2d(device: &wgpu::Device, label: &str, w: u32, h: u32, mips: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn write_texture(queue: &wgpu::Queue, tex: &wgpu::Texture, mip: u32, img: &image::RgbaImage) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: tex,
            mip_level: mip,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        img.as_raw(),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(img.width() * 4),
            rows_per_image: Some(img.height()),
        },
        wgpu::Extent3d {
            width: img.width(),
            height: img.height(),
            depth_or_array_layers: 1,
        },
    );
}

fn premultiplied(c: [f32; 4]) -> [f32; 4] {
    [c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]]
}

impl CanvasRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> CanvasRenderer {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("canvas"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let slots = Slots::new(device);
        let pool = Pool::new(device, format);
        let fx = FxPipelines::new(device, format, &slots, &pool);
        let tex_layout = |label| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
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
            })
        };
        let ramp_layout = tex_layout("ramp");
        let img_layout = tex_layout("image");
        let ramp_tex = texture_2d(device, "gradient ramps", tess::ATLAS_W, tess::ATLAS_H, 1);
        let ramp_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ramp"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let ramp_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ramp"),
            layout: &ramp_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &ramp_tex.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&ramp_sampler),
                },
            ],
        });
        let img_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("image"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let blank = texture_2d(device, "blank", 1, 1, 1);
        let blank_img = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blank image"),
            layout: &img_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &blank.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&img_sampler),
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("canvas"),
            bind_group_layouts: &[
                Some(&slots.globals_layout),
                Some(&ramp_layout),
                Some(&img_layout),
            ],
            immediate_size: 0,
        });
        let pipelines = (0..5)
            .map(|key| {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("canvas"),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs"),
                        compilation_options: Default::default(),
                        buffers: &[Some(wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<Vertex>() as u64,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &wgpu::vertex_attr_array![
                                0 => Float32x2, 1 => Float32x4, 2 => Float32x2, 3 => Uint32
                            ],
                        })],
                    },
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState {
                        count: MSAA,
                        mask: !0,
                        alpha_to_coverage_enabled: false,
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend: Some(blend_state(key)),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                })
            })
            .collect();
        CanvasRenderer {
            pipelines,
            fx,
            slots,
            pool,
            ramp_tex,
            ramp_bg,
            ramp_version: 0,
            img_layout,
            img_sampler,
            blank_img,
            images: HashMap::new(),
            world: None,
            msaa: None,
            canvas_copy: None,
            screen: None,
            format,
            uploads: 0,
            last_passes: 0,
        }
    }

    fn msaa_view(&mut self, device: &wgpu::Device, size: [u32; 2]) -> wgpu::TextureView {
        if self.msaa.as_ref().is_none_or(|(_, s)| *s != size) {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("canvas msaa"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: MSAA,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            self.msaa = Some((tex.create_view(&Default::default()), size));
        }
        self.msaa.as_ref().unwrap().0.clone()
    }

    fn canvas_copy_view(&mut self, device: &wgpu::Device, size: [u32; 2]) -> wgpu::TextureView {
        if self.canvas_copy.as_ref().is_none_or(|(_, _, s)| *s != size) {
            let (view, bind) = self.pool.texture(device, size, 1);
            self.canvas_copy = Some((view, bind.expect("single-sampled"), size));
        }
        self.canvas_copy.as_ref().unwrap().0.clone()
    }

    fn sync_ramps(&mut self, queue: &wgpu::Queue) {
        let a = tess::atlas().lock().unwrap();
        if a.version == self.ramp_version {
            return;
        }
        self.ramp_version = a.version;
        let img = image::RgbaImage::from_raw(tess::ATLAS_W, tess::ATLAS_H, a.pixels.clone())
            .expect("atlas size");
        write_texture(queue, &self.ramp_tex, 0, &img);
    }

    fn image_bg(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, id: &str) {
        if self.images.contains_key(id) {
            return;
        }
        let Some(a) = assets::get(id) else { return };
        // Mip chain so minified images stay smooth.
        let mut levels = vec![a.rgba.clone()];
        while levels.last().unwrap().width() > 1 || levels.last().unwrap().height() > 1 {
            let l = levels.last().unwrap();
            let (w, h) = ((l.width() / 2).max(1), (l.height() / 2).max(1));
            levels.push(image::imageops::resize(
                l,
                w,
                h,
                image::imageops::FilterType::Triangle,
            ));
            if levels.len() > 14 {
                break;
            }
        }
        let tex = texture_2d(device, "image", a.width, a.height, levels.len() as u32);
        for (i, l) in levels.iter().enumerate() {
            write_texture(queue, &tex, i as u32, l);
        }
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("image"),
            layout: &self.img_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &tex.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.img_sampler),
                },
            ],
        });
        self.images.insert(id.to_string(), bg);
    }

    fn sync_images(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, ops: &[Op]) {
        for op in ops {
            match op {
                Op::Item(it) => {
                    for p in it.pieces.iter() {
                        if let Some(id) = &p.image {
                            self.image_bg(device, queue, id);
                        }
                    }
                }
                Op::Blur(b) => self.sync_images(device, queue, &b.content),
                Op::Shadow(_) | Op::Backdrop(_) => {}
            }
        }
    }

    fn sync_world(&mut self, device: &wgpu::Device, ops: &Arc<Vec<Op>>) {
        if (self.world.as_ref()).is_some_and(|w| Arc::ptr_eq(&w.ops, ops)) {
            return;
        }
        let mut sig = Vec::new();
        signature(ops, &mut sig);
        if let Some(w) = &mut self.world
            && w.signature == sig
        {
            w.ops = ops.clone();
            return;
        }
        self.uploads += 1;
        let mut b = Builder::default();
        let steps = b.steps(ops);
        let mk = |data: &[u8], usage, label| {
            (!data.is_empty()).then(|| {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: data,
                    usage,
                })
            })
        };
        let vertex = wgpu::BufferUsages::VERTEX;
        self.world = Some(World {
            ops: ops.clone(),
            signature: sig,
            vb: mk(bytemuck::cast_slice(&b.vertices), vertex, "world vertices"),
            ib: mk(
                bytemuck::cast_slice(&b.indices),
                wgpu::BufferUsages::INDEX,
                "world indices",
            ),
            shadows: mk(bytemuck::cast_slice(&b.shadows), vertex, "box shadows"),
            steps,
            blurs: b.blurs,
            backdrops: b.backdrops,
        });
    }

    /// Record the passes that clear `target` and draw a canvas frame.
    /// Logical px × `scale` = physical px.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        size: [u32; 2],
        scale: f32,
        frame: &CanvasFrame,
        clear: [f64; 4],
    ) {
        let cam = frame.camera;
        self.last_passes = 0;
        self.sync_ramps(queue);
        self.sync_images(device, queue, &frame.ops);
        self.sync_world(device, &frame.ops);
        let world = self.world.take().expect("synced above");
        // A blur takes its target, two blur passes and the paint. A background blur takes a
        // slot per halving, two blur passes and the paint.
        let slots = 4 + 4 * world.blurs.len() + (MAX_HALVINGS + 3) * world.backdrops.len();
        self.slots.begin(device, slots as u32);
        let world_globals = Globals {
            size_px: [size[0] as f32, size[1] as f32],
            scale,
            zoom: cam.zoom as f32,
            pan: [cam.pan.x as f32, cam.pan.y as f32],
            offset: [frame.offset.x as f32, frame.offset.y as f32],
        };
        let screen_globals = Globals {
            zoom: 1.0,
            pan: [0.0; 2],
            offset: [0.0; 2],
            ..world_globals
        };
        let scale = scale as f64;
        let k = cam.zoom * scale;
        let vp = frame.viewport;
        let main = Target {
            size,
            map: Map {
                k,
                t: (frame.offset - cam.pan * cam.zoom) * scale,
            },
            visible: Rect::new(vp.x * scale, vp.y * scale, vp.w * scale, vp.h * scale),
            globals: self.slots.push(&world_globals),
            main: true,
        };
        let screen_slot = self.slots.push(&screen_globals);

        // Screen-space batches (underlay + overlay) are small: rebuilt every frame.
        let mut sverts: Vec<Vertex> = Vec::new();
        let mut sidx: Vec<u32> = Vec::new();
        let full = Rect::new(0.0, 0.0, size[0] as f64, size[1] as f64);
        let mut chrome = |list: &[super::mesh::Batch], out: &mut Vec<Cmd>| {
            for b in list {
                let base = sverts.len() as u32;
                let start = sidx.len() as u32;
                sverts.extend_from_slice(&b.mesh.vertices);
                sidx.extend(b.mesh.indices.iter().map(|i| i + base));
                let px = b.clip.map_or(full, |c| {
                    Rect::new(c.x * scale, c.y * scale, c.w * scale, c.h * scale)
                });
                if let Some(scissor) = main.scissor(&px) {
                    out.push(Cmd::Draw {
                        range: start..sidx.len() as u32,
                        scissor,
                        blend: 0,
                        image: None,
                        globals: screen_slot,
                        screen: true,
                    });
                }
            }
        };
        let mut cmds = Vec::new();
        chrome(&frame.under, &mut cmds);
        self.resolve(device, encoder, &world, &world.steps, &main, &mut cmds);
        chrome(&frame.over, &mut cmds);
        self.screen = (!sidx.is_empty()).then(|| {
            (
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("screen vertices"),
                    contents: bytemuck::cast_slice(&sverts),
                    usage: wgpu::BufferUsages::VERTEX,
                }),
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("screen indices"),
                    contents: bytemuck::cast_slice(&sidx),
                    usage: wgpu::BufferUsages::INDEX,
                }),
            )
        });

        let msaa = self.msaa_view(device, size);
        let clear = wgpu::Color {
            r: clear[0],
            g: clear[1],
            b: clear[2],
            a: clear[3],
        };
        let mut start = 0;
        let mut carried: Option<Cmd> = None;
        loop {
            let end = cmds[start..]
                .iter()
                .position(|c| matches!(c, Cmd::Backdrop { .. }))
                .map_or(cmds.len(), |i| start + i);
            let more = end < cmds.len();
            let copy = more.then(|| self.canvas_copy_view(device, size));
            let at = Attachments {
                view: &msaa,
                resolve: copy.as_ref().unwrap_or(target),
                load: if start == 0 {
                    wgpu::LoadOp::Clear(clear)
                } else {
                    wgpu::LoadOp::Load
                },
                store: if more {
                    wgpu::StoreOp::Store
                } else {
                    wgpu::StoreOp::Discard
                },
            };
            let list = carried.iter().chain(&cmds[start..end]);
            self.pass(encoder, &world, list, size, &at);
            // The pass that paints the last background blur is recorded, so its texture is
            // free for the next one.
            if let Some(Cmd::Paint { tex, .. }) = carried {
                self.pool.release(tex);
            }
            if !more {
                break;
            }
            let Cmd::Backdrop { plan, shown } = &cmds[end] else {
                unreachable!()
            };
            carried = Some(self.backdrop(device, encoder, &world.backdrops[*plan], shown, &main));
            start = end + 1;
        }
        self.slots.flush(queue);
        self.pool.end_frame();
        self.world = Some(world);
    }

    /// Turn steps into the draws of a pass on `tgt`. Blurs record their own passes here, so
    /// their textures are ready when the pass runs.
    fn resolve(
        &mut self,
        device: &wgpu::Device,
        enc: &mut wgpu::CommandEncoder,
        world: &World,
        steps: &[Step],
        tgt: &Target,
        out: &mut Vec<Cmd>,
    ) {
        let mut jobs = Vec::new();
        collect(world, steps, tgt, &mut jobs);
        let painted = self.bake(device, enc, world, jobs, tgt);
        emit(world, steps, tgt, &painted, out);
    }

    /// Share three passes per atlas page across all its blurs. Return composite draws by plan.
    fn bake(
        &mut self,
        device: &wgpu::Device,
        enc: &mut wgpu::CommandEncoder,
        world: &World,
        jobs: Vec<Job>,
        tgt: &Target,
    ) -> HashMap<usize, Cmd> {
        let mut painted = HashMap::new();
        if jobs.is_empty() {
            return painted;
        }
        let sizes: Vec<[u32; 2]> = jobs.iter().map(|j| j.size).collect();
        let pages: Vec<Page> = pack(&sizes)
            .into_iter()
            .map(|p| Page {
                size: p.size,
                tex: self.pool.acquire(device, p.size, 1),
                jobs: p.rects,
            })
            .collect();
        for page in &pages {
            let mut cmds = Vec::new();
            for (j, at) in &page.jobs {
                let job = &jobs[*j];
                let plan = &world.blurs[job.plan];
                let at_px = v2(at[0] as f64, at[1] as f64);
                let map = Map {
                    k: tgt.map.k / job.down,
                    t: (tgt.map.t + plan.offset * tgt.map.k - job.origin) / job.down + at_px,
                };
                let inner = Target {
                    size: page.size,
                    map,
                    visible: Rect::new(at_px.x, at_px.y, job.size[0] as f64, job.size[1] as f64),
                    globals: self.slots.push(&map.globals(page.size)),
                    main: false,
                };
                self.resolve(device, enc, world, &plan.steps, &inner, &mut cmds);
            }
            let msaa = self.pool.acquire(device, page.size, MSAA);
            let at = Attachments {
                view: &self.pool.get(msaa).view.clone(),
                resolve: &self.pool.get(page.tex).view.clone(),
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Discard,
            };
            self.pass(enc, world, cmds.iter(), page.size, &at);
            self.pool.release(msaa);
            // The pass is recorded, so the pages it sampled are free for later passes.
            for c in &cmds {
                if let Cmd::Paint { tex, .. } = c {
                    self.pool.release(*tex);
                }
            }

            let alloc = self.pool.get(page.tex).alloc;
            let sigma = |j: &Job| world.blurs[j.plan].sigma * tgt.map.k / j.down;
            if page.jobs.iter().any(|(j, _)| sigma(&jobs[*j]) * 3.0 >= 0.5) {
                let scratch = self.pool.acquire(device, page.size, 1);
                for (from, to, dir) in [(page.tex, scratch, 0), (scratch, page.tex, 1)] {
                    let parts: Vec<BlurPart> = (page.jobs.iter())
                        .map(|(j, at)| BlurPart {
                            src: Source::pool(from, *at, jobs[*j].size, alloc),
                            at: *at,
                            size: jobs[*j].size,
                            sigma: sigma(&jobs[*j]),
                            dir: Some(dir),
                        })
                        .collect();
                    self.blur_pass(enc, to, &parts);
                }
                self.pool.release(scratch);
            }

            for (j, at) in &page.jobs {
                let job = &jobs[*j];
                let plan = &world.blurs[job.plan];
                let (mode, color, mask) = match &plan.paint {
                    PlanPaint::Content => (MODE_CONTENT, [0.0; 4], None),
                    PlanPaint::Tint(c) => (MODE_TINT, premultiplied(*c), None),
                    PlanPaint::Inside { color, mask } => {
                        (MODE_INSIDE, premultiplied(*color), Some(mask.clone()))
                    }
                };
                let [aw, ah] = alloc.map(|a| a as f64);
                let src = Source::pool(page.tex, *at, job.size, alloc);
                let x = job.origin.x - at[0] as f64 * job.down;
                let y = job.origin.y - at[1] as f64 * job.down;
                let [w, h] = job.size.map(|s| s as f64 * job.down);
                let fx = self.slots.push(&FxParams {
                    rect: [x, y, 1.0 / (job.down * aw), 1.0 / (job.down * ah)].map(|v| v as f32),
                    clamp: src.valid_uv(),
                    color,
                    quad: [job.origin.x, job.origin.y, w, h].map(|v| v as f32),
                    params: [mode, tgt.size[0] as f32, tgt.size[1] as f32, 0.0],
                });
                if let Some(scissor) = tgt.scissor(&job.shown) {
                    let paint = Cmd::Paint {
                        tex: page.tex,
                        fx,
                        scissor,
                        mask,
                        mix: None,
                        globals: tgt.globals,
                    };
                    painted.insert(job.plan, paint);
                }
            }
        }
        painted
    }

    fn backdrop(
        &mut self,
        device: &wgpu::Device,
        enc: &mut wgpu::CommandEncoder,
        plan: &BackdropPlan,
        shown: &Rect,
        tgt: &Target,
    ) -> Cmd {
        let sigma = plan.sigma * tgt.map.k;
        let [tw, th] = tgt.size.map(|s| s as f64);
        // The blur reads past the layer, but not past the canvas.
        let x0 = (shown.x - 3.0 * sigma).floor().clamp(0.0, tw - 1.0);
        let y0 = (shown.y - 3.0 * sigma).floor().clamp(0.0, th - 1.0);
        let w = ((shown.right() + 3.0 * sigma).ceil().clamp(1.0, tw) - x0).max(1.0);
        let h = ((shown.bottom() + 3.0 * sigma).ceil().clamp(1.0, th) - y0).max(1.0);
        let mut left = (sigma / MAX_TEXEL_SIGMA).max(1.0);
        let first = left.min(2.0);
        let largest = [(w / first).ceil() as u32, (h / first).ceil() as u32];
        let mut to = self.pool.acquire(device, largest, 1);
        let mut spare = self.pool.acquire(device, largest, 1);
        let alloc = self.pool.get(to).alloc;
        let mut src = Source {
            tex: Tex::Canvas,
            at: [x0, y0],
            size: [w, h],
            alloc: tgt.size,
        };
        let mut size = [w as u32, h as u32];
        let whole = |src, size, sigma, dir| BlurPart {
            src,
            at: [0; 2],
            size,
            sigma,
            dir,
        };
        // Halve until the blur is narrow enough, so every texel of the canvas still counts.
        for _ in 0..MAX_HALVINGS {
            if left <= 1.0 + 1e-6 || size == [1, 1] {
                break;
            }
            let f = left.min(2.0);
            size = size.map(|s| ((s as f64 / f).ceil() as u32).max(1));
            self.blur_pass(enc, to, &[whole(src, size, 0.0, None)]);
            src = Source::pool(to, [0; 2], size, alloc);
            std::mem::swap(&mut to, &mut spare);
            left /= f;
        }
        let across = sigma * size[0] as f64 / w;
        let down = sigma * size[1] as f64 / h;
        self.blur_pass(enc, to, &[whole(src, size, across, Some(0))]);
        let src = Source::pool(to, [0; 2], size, alloc);
        self.blur_pass(enc, spare, &[whole(src, size, down, Some(1))]);
        self.pool.release(to);
        let result = Source::pool(spare, [0; 2], size, alloc);
        let [aw, ah] = alloc.map(|a| a as f64);
        let fx = self.slots.push(&FxParams {
            rect: [x0, y0, size[0] as f64 / (w * aw), size[1] as f64 / (h * ah)].map(|v| v as f32),
            clamp: result.valid_uv(),
            color: [0.0; 4],
            quad: [0.0; 4],
            params: [MODE_CONTENT, tw as f32, th as f32, 0.0],
        });
        Cmd::Paint {
            tex: spare,
            fx,
            scissor: tgt.scissor(shown).unwrap_or([0, 0, 1, 1]),
            mask: Some(plan.mask.clone()),
            mix: Some(plan.opacity.clamp(0.0, 1.0)),
            globals: tgt.globals,
        }
    }

    fn blur_pass(&mut self, enc: &mut wgpu::CommandEncoder, to: usize, parts: &[BlurPart]) {
        let slots: Vec<u32> = (parts.iter())
            .map(|p| {
                let mut step = [0.0; 2];
                let mut radius = 0.0;
                if let Some(d) = p.dir {
                    step[d] = 1.0 / p.src.alloc[d] as f64;
                    radius = (p.sigma * 3.0).ceil().min(32.0);
                }
                self.slots.push(&FxParams {
                    rect: p.src.uv(),
                    clamp: p.src.valid_uv(),
                    color: [step[0], step[1], p.sigma, radius].map(|v| v as f32),
                    quad: [p.at[0] as f32, p.at[1] as f32, 0.0, 0.0],
                    params: [0.0, 1.0 / p.size[0] as f32, 1.0 / p.size[1] as f32, 0.0],
                })
            })
            .collect();
        self.last_passes += 1;
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("blur"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.pool.get(to).view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.fx.blur);
        pass.set_bind_group(0, &self.slots.globals, &[0]);
        for (p, fx) in parts.iter().zip(slots) {
            let [x, y, w, h] = [p.at[0], p.at[1], p.size[0], p.size[1]].map(|v| v as f32);
            pass.set_viewport(x, y, w, h, 0.0, 1.0);
            pass.set_bind_group(1, &self.slots.fx, &[fx]);
            pass.set_bind_group(2, self.texture(p.src.tex), &[]);
            pass.draw(0..4, 0..1);
        }
    }

    fn texture(&self, tex: Tex) -> &wgpu::BindGroup {
        match tex {
            Tex::Pool(i) => self.pool.get(i).bind.as_ref().expect("single-sampled"),
            Tex::Canvas => &self.canvas_copy.as_ref().expect("made before the pass").1,
        }
    }

    /// Record one multisampled pass that draws `cmds` into the top-left `size` px.
    fn pass<'c>(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        world: &World,
        cmds: impl Iterator<Item = &'c Cmd>,
        size: [u32; 2],
        at: &Attachments,
    ) {
        self.last_passes += 1;
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("canvas"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: at.view,
                depth_slice: None,
                resolve_target: Some(at.resolve),
                ops: wgpu::Operations {
                    load: at.load,
                    store: at.store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_viewport(0.0, 0.0, size[0] as f32, size[1] as f32, 0.0, 1.0);
        let world_buffers = world.vb.as_ref().zip(world.ib.as_ref());
        let screen_buffers = self.screen.as_ref().map(|(v, i)| (v, i));
        #[derive(PartialEq, Clone, Copy)]
        enum Bound {
            None,
            Canvas(usize),
            Shadow,
            Quad,
            Mask(bool),
        }
        let mut bound = Bound::None;
        for cmd in cmds {
            match cmd {
                Cmd::Draw {
                    range,
                    scissor,
                    blend,
                    image,
                    globals,
                    screen,
                } => {
                    let buffers = if *screen {
                        screen_buffers
                    } else {
                        world_buffers
                    };
                    let Some((vb, ib)) = buffers else { continue };
                    if range.is_empty() {
                        continue;
                    }
                    if bound != Bound::Canvas(*blend) {
                        pass.set_pipeline(&self.pipelines[*blend]);
                        pass.set_bind_group(1, &self.ramp_bg, &[]);
                        bound = Bound::Canvas(*blend);
                    }
                    pass.set_bind_group(0, &self.slots.globals, &[*globals]);
                    let img = image
                        .as_ref()
                        .and_then(|i| self.images.get(i))
                        .unwrap_or(&self.blank_img);
                    pass.set_bind_group(2, img, &[]);
                    pass.set_vertex_buffer(0, vb.slice(..));
                    pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                    let [x, y, w, h] = *scissor;
                    pass.set_scissor_rect(x, y, w, h);
                    pass.draw_indexed(range.clone(), 0, 0..1);
                }
                Cmd::Shadows {
                    range,
                    scissor,
                    globals,
                } => {
                    let Some(instances) = &world.shadows else {
                        continue;
                    };
                    if bound != Bound::Shadow {
                        pass.set_pipeline(&self.fx.shadow);
                        bound = Bound::Shadow;
                    }
                    pass.set_bind_group(0, &self.slots.globals, &[*globals]);
                    pass.set_vertex_buffer(0, instances.slice(..));
                    let [x, y, w, h] = *scissor;
                    pass.set_scissor_rect(x, y, w, h);
                    pass.draw(0..4, range.clone());
                }
                Cmd::Paint {
                    tex,
                    fx,
                    scissor,
                    mask,
                    mix,
                    globals,
                } => {
                    let kind = match mask {
                        Some(_) => Bound::Mask(mix.is_some()),
                        None => Bound::Quad,
                    };
                    if let Some(m) = *mix {
                        pass.set_blend_constant(wgpu::Color {
                            r: m,
                            g: m,
                            b: m,
                            a: m,
                        });
                    }
                    if bound != kind {
                        pass.set_pipeline(match kind {
                            Bound::Mask(mix) => &self.fx.mask[mix as usize],
                            _ => &self.fx.quad,
                        });
                        bound = kind;
                    }
                    pass.set_bind_group(0, &self.slots.globals, &[*globals]);
                    pass.set_bind_group(1, &self.slots.fx, &[*fx]);
                    pass.set_bind_group(2, self.texture(Tex::Pool(*tex)), &[]);
                    let [x, y, w, h] = *scissor;
                    pass.set_scissor_rect(x, y, w, h);
                    match mask {
                        Some(range) => {
                            let Some((vb, ib)) = world_buffers else {
                                continue;
                            };
                            pass.set_vertex_buffer(0, vb.slice(..));
                            pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                            pass.draw_indexed(range.clone(), 0, 0..1);
                        }
                        None => pass.draw(0..4, 0..1),
                    }
                }
                Cmd::Backdrop { .. } => {}
            }
        }
    }
}

fn blur_mode(i: usize, plan: &BlurPlan, tgt: &Target) -> BlurMode {
    let k = tgt.map.k;
    let sigma = plan.sigma * k;
    let reach = 3.0 * sigma + 1.0;
    let content = tgt.map.rect(&plan.bounds.translate(plan.offset));
    let Some(shown) = tgt.shown(content.inflate(reach), &plan.clip) else {
        return BlurMode::Skip;
    };
    match plan.paint {
        PlanPaint::Content if sigma < MIN_SIGMA_PX => return BlurMode::Inline,
        // A shadow that moves and spreads less than a pixel hides under its layer.
        PlanPaint::Tint(_) | PlanPaint::Inside { .. }
            if 3.0 * sigma + (plan.offset.len() + plan.spread) * k < 0.5 =>
        {
            return BlurMode::Skip;
        }
        _ => {}
    }
    // What is shown needs the content around it, as far as the blur reaches.
    let Some(region) = shown.inflate(reach).intersection(&content.inflate(reach)) else {
        return BlurMode::Skip;
    };
    let origin = v2(region.x.floor(), region.y.floor());
    let (w, h) = (
        region.right().ceil() - origin.x,
        region.bottom().ceil() - origin.y,
    );
    let down = (sigma / MAX_TEXEL_SIGMA)
        .max(w.max(h) / MAX_BLUR_EDGE)
        .max(1.0);
    BlurMode::Offscreen(Job {
        plan: i,
        shown,
        origin,
        down,
        size: [w, h].map(|s| (s / down).ceil().max(1.0) as u32),
    })
}

fn collect(world: &World, steps: &[Step], tgt: &Target, jobs: &mut Vec<Job>) {
    for step in steps {
        if let Step::Blur(i) = step {
            let plan = &world.blurs[*i];
            match blur_mode(*i, plan, tgt) {
                BlurMode::Offscreen(job) => jobs.push(job),
                BlurMode::Inline => collect(world, &plan.steps, tgt, jobs),
                BlurMode::Skip => {}
            }
        }
    }
}

fn emit(
    world: &World,
    steps: &[Step],
    tgt: &Target,
    painted: &HashMap<usize, Cmd>,
    out: &mut Vec<Cmd>,
) {
    for step in steps {
        match step {
            Step::Draw(d) => {
                if let Some(scissor) = tgt.clip(&d.clip) {
                    out.push(Cmd::Draw {
                        range: d.range.clone(),
                        scissor,
                        blend: d.blend,
                        image: d.image.clone(),
                        globals: tgt.globals,
                        screen: false,
                    });
                }
            }
            Step::Shadows { range, clip } => {
                if let Some(scissor) = tgt.clip(clip) {
                    out.push(Cmd::Shadows {
                        range: range.clone(),
                        scissor,
                        globals: tgt.globals,
                    });
                }
            }
            Step::Blur(i) => {
                let plan = &world.blurs[*i];
                match painted.get(i) {
                    Some(Cmd::Paint {
                        tex,
                        fx,
                        scissor,
                        mask,
                        mix,
                        globals,
                    }) => out.push(Cmd::Paint {
                        tex: *tex,
                        fx: *fx,
                        scissor: *scissor,
                        mask: mask.clone(),
                        mix: *mix,
                        globals: *globals,
                    }),
                    _ if matches!(blur_mode(*i, plan, tgt), BlurMode::Inline) => {
                        emit(world, &plan.steps, tgt, painted, out)
                    }
                    _ => {}
                }
            }
            Step::Backdrop(i) => {
                let b = &world.backdrops[*i];
                let shown = tgt.shown(tgt.map.rect(&b.bounds), &b.clip);
                if let Some(shown) = shown
                    && tgt.main
                    && b.sigma * tgt.map.k >= MIN_SIGMA_PX
                    && b.opacity > 0.0
                    && !b.mask.is_empty()
                {
                    out.push(Cmd::Backdrop { plan: *i, shown });
                }
            }
        }
    }
}

/// A device without a window, for tests, screenshots and PNG export.
#[derive(Clone)]
pub struct Headless {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter_info: wgpu::AdapterInfo,
}

impl Headless {
    pub fn new() -> Result<Headless, String> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: None,
            ..Default::default()
        }))
        .map_err(|e| format!("no GPU adapter available: {e}"))?;
        let adapter_info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("sketch-rs headless"),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| format!("failed to create device: {e}"))?;
        Ok(Headless {
            device,
            queue,
            adapter_info,
        })
    }

    /// Create a render target texture of `size` physical px.
    pub fn target(&self, size: [u32; 2]) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OFFSCREEN_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    /// Copy a texture back to the CPU as RGBA8.
    pub fn read(&self, tex: &wgpu::Texture) -> image::RgbaImage {
        let (w, h) = (tex.width(), tex.height());
        let row = (w * 4).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll");
        let data = slice.get_mapped_range().expect("mapped readback buffer");
        let mut img = image::RgbaImage::new(w, h);
        for y in 0..h {
            let src = &data[(y * row) as usize..(y * row + w * 4) as usize];
            img.as_mut()[(y * w * 4) as usize..((y + 1) * w * 4) as usize].copy_from_slice(src);
        }
        img
    }

    /// Render a canvas frame into a new image of `size` physical px.
    pub fn render_frame(
        &self,
        frame: &CanvasFrame,
        size: [u32; 2],
        scale: f32,
        clear: [f64; 4],
    ) -> image::RgbaImage {
        let tex = self.target(size);
        let view = tex.create_view(&Default::default());
        let mut r = CanvasRenderer::new(&self.device, OFFSCREEN_FORMAT);
        let mut enc = self.device.create_command_encoder(&Default::default());
        r.render(
            &self.device,
            &self.queue,
            &mut enc,
            &view,
            size,
            scale,
            frame,
            clear,
        );
        self.queue.submit([enc.finish()]);
        self.read(&tex)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_rects_stay_on_their_page_and_apart() {
        let sizes: Vec<[u32; 2]> = (0..400u32)
            .map(|i| [20 + i % 37 * 9, 10 + i % 23 * 11])
            .chain([[2048, 2048], [1, 1]])
            .collect();
        let pages = pack(&sizes);
        assert!(pages.len() > 1, "the full-page rect needs its own");
        let mut seen = vec![false; sizes.len()];
        for page in &pages {
            assert!(page.size[0] <= PAGE_EDGE && page.size[1] <= PAGE_EDGE);
            let rects: Vec<Rect> = (page.rects.iter())
                .map(|(i, at)| {
                    seen[*i] = true;
                    let [w, h] = sizes[*i].map(|s| s as f64);
                    Rect::new(at[0] as f64, at[1] as f64, w, h)
                })
                .collect();
            for (n, a) in rects.iter().enumerate() {
                assert!(a.right() <= page.size[0] as f64 && a.bottom() <= page.size[1] as f64);
                let overlap = rects[..n].iter().find(|b| {
                    a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
                });
                assert!(overlap.is_none(), "{a:?} overlaps {overlap:?}");
            }
        }
        assert!(seen.iter().all(|s| *s), "every rect is placed");
    }
}
