//! wgpu 渲染器：天空查找表 → 陰影 → 主場景（MSAA、反向 Z）→ 泛光 → 色調映射。

use crate::lod::{frustum_planes, Lod, MinMax, NodeInstance};
use crate::meshes::{self, MeshVertex, RoadVertex};
use crate::shaders;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;
use worldgen::fine::FineData;
use worldgen::World;

pub const HDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
pub const MSAA: u32 = 4;
pub const SHADOW_SIZE: u32 = 2048;
const LUT_W: u32 = 256;
const LUT_H: u32 = 128;
const BLOOM_LEVELS: usize = 6;
const MAX_NODES: usize = 4096;
const MAX_CAR_PARTS: usize = 32;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
pub struct Globals {
    pub view_proj: [[f32; 4]; 4],
    pub inv_view_proj: [[f32; 4]; 4],
    pub shadow_mat: [[f32; 4]; 4],
    pub cam_pos: [f32; 4],
    pub light_dir: [f32; 4],
    pub light_color: [f32; 4],
    pub sun_dir: [f32; 4],
    pub ambient: [f32; 4],
    pub fog: [f32; 4],
    pub screen: [f32; 4],
    pub grid: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct BuildingInstance {
    pub pos: [f32; 4],
    pub size: [f32; 4],
    pub kind: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct PartInstance {
    pub m: [[f32; 4]; 4],
    pub color: [f32; 4],
}

struct Mesh {
    vb: wgpu::Buffer,
    ib: wgpu::Buffer,
    count: u32,
}

fn mesh<V: Pod, I: Pod>(device: &wgpu::Device, v: &[V], i: &[I]) -> Mesh {
    Mesh {
        vb: device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(v), usage: wgpu::BufferUsages::VERTEX }),
        ib: device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(i), usage: wgpu::BufferUsages::INDEX }),
        count: i.len() as u32,
    }
}

struct Targets {
    hdr_msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
    hdr: wgpu::TextureView,
    bloom: Vec<(wgpu::TextureView, u32, u32)>,
    w: u32,
    h: u32,
    down_bgs: Vec<wgpu::BindGroup>,
    up_bgs: Vec<wgpu::BindGroup>,
    tonemap_bg: wgpu::BindGroup,
}

pub struct FrameParams {
    pub view_proj: Mat4,
    pub cam: Vec3,
    pub time: f32,
    pub light_dir: Vec3,
    pub light_color: Vec3,
    pub sun_dir: Vec3,
    pub night: f32,
    pub exposure: f32,
    pub fog_density: f32,
    pub cloud_cover: f32,
    pub parts: Vec<PartInstance>,
}

pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
    pub adapter_info: String,
    manual_gamma: bool,
    globals_buf: wgpu::Buffer,
    post_buf: wgpu::Buffer,
    scene_bg: wgpu::BindGroup,
    globals_bg: wgpu::BindGroup,
    sky_bg: wgpu::BindGroup,
    lut_view: wgpu::TextureView,
    shadow_view: wgpu::TextureView,
    post_bgl: wgpu::BindGroupLayout,
    lin_samp: wgpu::Sampler,
    p_lut: wgpu::RenderPipeline,
    p_sky: wgpu::RenderPipeline,
    p_terrain: wgpu::RenderPipeline,
    p_water: wgpu::RenderPipeline,
    p_ocean: wgpu::RenderPipeline,
    p_road: wgpu::RenderPipeline,
    p_building: wgpu::RenderPipeline,
    p_building_shadow: wgpu::RenderPipeline,
    p_car: wgpu::RenderPipeline,
    p_car_shadow: wgpu::RenderPipeline,
    p_down_first: wgpu::RenderPipeline,
    p_down: wgpu::RenderPipeline,
    p_up: wgpu::RenderPipeline,
    p_tonemap: wgpu::RenderPipeline,
    patch: Mesh,
    cube_b: Mesh,
    cube_c: Mesh,
    cyl: Mesh,
    roads: Mesh,
    buildings: wgpu::Buffer,
    building_count: u32,
    nodes_buf: wgpu::Buffer,
    water_nodes_buf: wgpu::Buffer,
    parts_buf: wgpu::Buffer,
    targets: Targets,
    pub lod: Lod,
    minmax: MinMax,
    wet: MinMax,
    world_size: f32,
    grid: [f32; 4],
    pub render_scale: f32,
    pub last_nodes: usize,
    node_scratch: Vec<NodeInstance>,
    water_scratch: Vec<NodeInstance>,
    /// 除錯截圖：從 GPU 讀回畫面
    pub capture_request: bool,
    capture: Option<(wgpu::Buffer, u32, u32, u32, std::rc::Rc<std::cell::Cell<bool>>)>,
}

fn tex2d(device: &wgpu::Device, w: u32, h: u32, format: wgpu::TextureFormat, usage: wgpu::TextureUsages, samples: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

fn upload(queue: &wgpu::Queue, tex: &wgpu::Texture, data: &[u8], w: u32, h: u32, bpp: u32) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        data,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * bpp), rows_per_image: Some(h) },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
}

fn bgl_entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT, ty, count: None }
}

fn tex_ty(filterable: bool) -> wgpu::BindingType {
    wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable },
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
    }
}

fn uniform_ty() -> wgpu::BindingType {
    wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }
}

struct PipeDesc<'a> {
    module: &'a wgpu::ShaderModule,
    vs: &'a str,
    fs: Option<&'a str>,
    buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
    layout: &'a wgpu::PipelineLayout,
    color: Option<wgpu::TextureFormat>,
    blend: Option<wgpu::BlendState>,
    depth: Option<(wgpu::TextureFormat, wgpu::CompareFunction, bool)>,
    samples: u32,
    cull: Option<wgpu::Face>,
    bias: wgpu::DepthBiasState,
}

fn pipeline(device: &wgpu::Device, d: PipeDesc) -> wgpu::RenderPipeline {
    let targets = [d.color.map(|f| wgpu::ColorTargetState { format: f, blend: d.blend, write_mask: wgpu::ColorWrites::ALL })];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(d.vs),
        layout: Some(d.layout),
        vertex: wgpu::VertexState { module: d.module, entry_point: Some(d.vs), compilation_options: Default::default(), buffers: d.buffers },
        primitive: wgpu::PrimitiveState { cull_mode: d.cull, ..Default::default() },
        depth_stencil: d.depth.map(|(format, cmp, write)| wgpu::DepthStencilState {
            format,
            depth_write_enabled: Some(write),
            depth_compare: Some(cmp),
            stencil: Default::default(),
            bias: d.bias,
        }),
        multisample: wgpu::MultisampleState { count: d.samples, mask: !0, alpha_to_coverage_enabled: false },
        fragment: d.fs.map(|fs| wgpu::FragmentState { module: d.module, entry_point: Some(fs), compilation_options: Default::default(), targets: if d.color.is_some() { &targets } else { &[] } }),
        multiview_mask: None,
        cache: None,
    })
}

impl Renderer {
    pub async fn new(canvas: web_sys::HtmlCanvasElement, world: &World, fine: &FineData, quality_high: bool) -> Result<Self, String> {
        let width = canvas.width().max(1);
        let height = canvas.height().max(1);
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::BROWSER_WEBGPU, ..wgpu::InstanceDescriptor::new_without_display_handle() });
        let surface = instance.create_surface(wgpu::SurfaceTarget::Canvas(canvas)).map_err(|e| format!("create_surface: {e}"))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, compatible_surface: Some(&surface), force_fallback_adapter: false, ..Default::default() })
            .await
            .map_err(|e| format!("找不到 WebGPU 介面卡：{e}"))?;
        let info = adapter.get_info();
        let adapter_info = format!("{} / {} / {:?}", info.vendor, info.name, info.backend);
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor { label: None, required_features: wgpu::Features::empty(), required_limits: wgpu::Limits::default(), ..Default::default() })
            .await
            .map_err(|e| format!("request_device: {e}"))?;
        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &config);
        let manual_gamma = !format.is_srgb();

        // ---------- 世界紋理 ----------
        let tb = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
        let fn_ = fine.n as u32;
        let wn = fine.water_n as u32;
        let height_tex = tex2d(&device, fn_, fn_, wgpu::TextureFormat::R32Float, tb, 1);
        upload(&queue, &height_tex, bytemuck::cast_slice(&fine.height), fn_, fn_, 4);
        let water_tex = tex2d(&device, wn, wn, wgpu::TextureFormat::R32Float, tb, 1);
        upload(&queue, &water_tex, bytemuck::cast_slice(&fine.water), wn, wn, 4);
        let cover_tex = tex2d(&device, wn, wn, wgpu::TextureFormat::Rgba8Unorm, tb, 1);
        upload(&queue, &cover_tex, &fine.cover, wn, wn, 4);
        let road_tex = tex2d(&device, fn_, fn_, wgpu::TextureFormat::R8Unorm, tb, 1);
        upload(&queue, &road_tex, &fine.road, fn_, fn_, 1);
        let rt = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let lut_tex = tex2d(&device, LUT_W, LUT_H, HDR, rt, 1);
        let shadow_tex = tex2d(&device, SHADOW_SIZE, SHADOW_SIZE, DEPTH, rt, 1);
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let lut_view = view(&lut_tex);
        let shadow_view = view(&shadow_tex);

        let lin_samp = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let lut_samp = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shadow_samp = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });

        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let post_buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("post"), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });

        // ---------- 綁定佈局 ----------
        let scene_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene"),
            entries: &[
                bgl_entry(0, uniform_ty()),
                bgl_entry(1, tex_ty(false)),
                bgl_entry(2, tex_ty(false)),
                bgl_entry(3, tex_ty(true)),
                bgl_entry(4, tex_ty(true)),
                bgl_entry(5, tex_ty(true)),
                bgl_entry(6, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                bgl_entry(7, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                bgl_entry(8, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison)),
            ],
        });
        let globals_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("globals"), entries: &[bgl_entry(0, uniform_ty())] });
        let sky_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky"),
            entries: &[bgl_entry(0, uniform_ty()), bgl_entry(1, tex_ty(true)), bgl_entry(2, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering))],
        });
        let post_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post"),
            entries: &[
                bgl_entry(0, tex_ty(true)),
                bgl_entry(1, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                bgl_entry(2, tex_ty(true)),
                bgl_entry(3, uniform_ty()),
            ],
        });
        let road_view = view(&road_tex);
        let scene_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene"),
            layout: &scene_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view(&height_tex)) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&view(&water_tex)) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&view(&cover_tex)) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&road_view) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&lut_view) },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::Sampler(&lut_samp) },
                wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(&shadow_view) },
                wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::Sampler(&shadow_samp) },
            ],
        });
        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &globals_bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals_buf.as_entire_binding() }],
        });
        let sky_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky"),
            layout: &sky_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&lut_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&lut_samp) },
            ],
        });

        // ---------- 著色器與管線 ----------
        let module = |src: String, label: &str| device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let m_sky = module(shaders::SKY.to_string(), "sky");
        let m_terrain = module(shaders::terrain(), "terrain");
        let m_obj = module(shaders::objects(), "objects");
        let m_post = module(shaders::POST.to_string(), "post");
        let layout = |bgls: &[&wgpu::BindGroupLayout]| {
            let v: Vec<Option<&wgpu::BindGroupLayout>> = bgls.iter().map(|b| Some(*b)).collect();
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &v, immediate_size: 0 })
        };
        let l_scene = layout(&[&scene_bgl]);
        let l_globals = layout(&[&globals_bgl]);
        let l_sky = layout(&[&sky_bgl]);
        let l_post = layout(&[&post_bgl]);

        let grid_layout = wgpu::VertexBufferLayout { array_stride: 8, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![2 => Float32x2] };
        let node_layout = wgpu::VertexBufferLayout { array_stride: 32, step_mode: wgpu::VertexStepMode::Instance, attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4] };
        let road_layout = wgpu::VertexBufferLayout { array_stride: 28, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x2] };
        let mesh_layout = wgpu::VertexBufferLayout { array_stride: 24, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3] };
        let bld_layout = wgpu::VertexBufferLayout { array_stride: 36, step_mode: wgpu::VertexStepMode::Instance, attributes: &wgpu::vertex_attr_array![2 => Float32x4, 3 => Float32x4, 4 => Uint32] };
        let part_layout = wgpu::VertexBufferLayout {
            array_stride: 80,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4],
        };

        let depth_main = Some((DEPTH, wgpu::CompareFunction::Greater, true));
        let no_bias = wgpu::DepthBiasState::default();
        let p_lut = pipeline(&device, PipeDesc { module: &m_sky, vs: "vs_full", fs: Some("fs_lut"), buffers: &[], layout: &l_globals, color: Some(HDR), blend: None, depth: None, samples: 1, cull: None, bias: no_bias });
        let p_sky = pipeline(&device, PipeDesc {
            module: &m_sky,
            vs: "vs_full",
            fs: Some("fs_sky"),
            buffers: &[],
            layout: &l_sky,
            color: Some(HDR),
            blend: None,
            depth: Some((DEPTH, wgpu::CompareFunction::GreaterEqual, false)),
            samples: MSAA,
            cull: None,
            bias: no_bias,
        });
        let p_terrain = pipeline(&device, PipeDesc {
            module: &m_terrain,
            vs: "vs_terrain",
            fs: Some("fs_terrain"),
            buffers: &[Some(grid_layout.clone()), Some(node_layout.clone())],
            layout: &l_scene,
            color: Some(HDR),
            blend: None,
            depth: depth_main,
            samples: MSAA,
            cull: None,
            bias: no_bias,
        });
        let p_water = pipeline(&device, PipeDesc {
            module: &m_terrain,
            vs: "vs_water",
            fs: Some("fs_water"),
            buffers: &[Some(grid_layout.clone()), Some(node_layout.clone())],
            layout: &l_scene,
            color: Some(HDR),
            blend: None,
            depth: depth_main,
            samples: MSAA,
            cull: None,
            bias: no_bias,
        });
        let p_ocean = pipeline(&device, PipeDesc {
            module: &m_terrain,
            vs: "vs_ocean",
            fs: Some("fs_ocean"),
            buffers: &[Some(grid_layout.clone())],
            layout: &l_scene,
            color: Some(HDR),
            blend: None,
            depth: depth_main,
            samples: MSAA,
            cull: None,
            bias: no_bias,
        });
        let p_road = pipeline(&device, PipeDesc {
            module: &m_obj,
            vs: "vs_road",
            fs: Some("fs_road"),
            buffers: &[Some(road_layout)],
            layout: &l_scene,
            color: Some(HDR),
            blend: None,
            depth: depth_main,
            samples: MSAA,
            cull: None,
            bias: wgpu::DepthBiasState { constant: 2, slope_scale: 1.0, clamp: 0.0 },
        });
        let p_building = pipeline(&device, PipeDesc {
            module: &m_obj,
            vs: "vs_building",
            fs: Some("fs_building"),
            buffers: &[Some(mesh_layout.clone()), Some(bld_layout.clone())],
            layout: &l_scene,
            color: Some(HDR),
            blend: None,
            depth: depth_main,
            samples: MSAA,
            cull: None,
            bias: no_bias,
        });
        let shadow_depth = Some((DEPTH, wgpu::CompareFunction::LessEqual, true));
        let shadow_bias = wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 };
        let p_building_shadow = pipeline(&device, PipeDesc {
            module: &m_obj,
            vs: "vs_building_shadow",
            fs: None,
            buffers: &[Some(mesh_layout.clone()), Some(bld_layout)],
            layout: &l_globals,
            color: None,
            blend: None,
            depth: shadow_depth,
            samples: 1,
            cull: None,
            bias: shadow_bias,
        });
        let p_car = pipeline(&device, PipeDesc {
            module: &m_obj,
            vs: "vs_car",
            fs: Some("fs_car"),
            buffers: &[Some(mesh_layout.clone()), Some(part_layout.clone())],
            layout: &l_scene,
            color: Some(HDR),
            blend: None,
            depth: depth_main,
            samples: MSAA,
            cull: None,
            bias: no_bias,
        });
        let p_car_shadow = pipeline(&device, PipeDesc {
            module: &m_obj,
            vs: "vs_car_shadow",
            fs: None,
            buffers: &[Some(mesh_layout), Some(part_layout)],
            layout: &l_globals,
            color: None,
            blend: None,
            depth: shadow_depth,
            samples: 1,
            cull: None,
            bias: shadow_bias,
        });
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent::OVER,
        };
        let post = |fs: &str, fmt: wgpu::TextureFormat, blend: Option<wgpu::BlendState>| {
            pipeline(&device, PipeDesc { module: &m_post, vs: "vs_full", fs: Some(fs), buffers: &[], layout: &l_post, color: Some(fmt), blend, depth: None, samples: 1, cull: None, bias: no_bias })
        };
        let p_down_first = post("fs_down_first", HDR, None);
        let p_down = post("fs_down", HDR, None);
        let p_up = post("fs_up", HDR, Some(additive));
        let p_tonemap = post("fs_tonemap", format, None);

        // ---------- 幾何 ----------
        let (gv, gi) = meshes::patch_grid();
        let patch = mesh(&device, &gv, &gi);
        let (cv, ci) = meshes::cube(0.0);
        let cube_b = mesh(&device, &cv, &ci);
        let (cv, ci) = meshes::cube(-0.5);
        let cube_c = mesh(&device, &cv, &ci);
        let (yv, yi) = meshes::cylinder(24);
        let cyl = mesh(&device, &yv, &yi);
        let (rv, ri) = meshes::road_mesh(world.roads.iter().chain(world.streets.iter()));
        let roads = mesh::<RoadVertex, u32>(&device, &rv, &ri);
        let binst: Vec<BuildingInstance> = world
            .buildings
            .iter()
            .map(|b| BuildingInstance { pos: [b.x, b.y, b.z, b.angle], size: [b.sx, b.sy, b.sz, (b.seed % 100_000) as f32], kind: b.kind as u32 })
            .collect();
        let buildings = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("buildings"),
            contents: if binst.is_empty() { &[0u8; 36] } else { bytemuck::cast_slice(&binst) },
            usage: wgpu::BufferUsages::VERTEX,
        });
        let vb = |size: usize| {
            device.create_buffer(&wgpu::BufferDescriptor { label: None, size: size as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })
        };
        let nodes_buf = vb(MAX_NODES * 32);
        let water_nodes_buf = vb(MAX_NODES * 32);
        let parts_buf = vb(MAX_CAR_PARTS * 80);

        let lod = Lod::new(world.size, if quality_high { 32.0 } else { 64.0 }, 2.5);
        let minmax = MinMax::new(fine.n, fine.cell, &fine.height);
        let wet_data: Vec<f32> = (0..fine.water_n * fine.water_n)
            .map(|k| if world.hydro.kind.data[k] != 0 { 1.0 } else { 0.0 })
            .collect();
        let wet = MinMax::new(fine.water_n, world.height.cell, &wet_data);
        let grid = [fine.cell, fine.n as f32, world.height.cell, fine.water_n as f32];

        let render_scale = if quality_high { 1.0 } else { 0.75 };
        let targets = Self::make_targets(&device, &post_bgl, &lin_samp, &post_buf, width, height, render_scale);
        let _ = MeshVertex { pos: [0.0; 3], normal: [0.0; 3] };
        Ok(Self {
            device,
            queue,
            surface,
            config,
            adapter_info,
            manual_gamma,
            globals_buf,
            post_buf,
            scene_bg,
            globals_bg,
            sky_bg,
            lut_view,
            shadow_view,
            post_bgl,
            lin_samp,
            p_lut,
            p_sky,
            p_terrain,
            p_water,
            p_ocean,
            p_road,
            p_building,
            p_building_shadow,
            p_car,
            p_car_shadow,
            p_down_first,
            p_down,
            p_up,
            p_tonemap,
            patch,
            cube_b,
            cube_c,
            cyl,
            roads,
            buildings,
            building_count: binst.len() as u32,
            nodes_buf,
            water_nodes_buf,
            parts_buf,
            targets,
            lod,
            minmax,
            wet,
            world_size: world.size,
            grid,
            render_scale,
            last_nodes: 0,
            node_scratch: Vec::new(),
            water_scratch: Vec::new(),
            capture_request: false,
            capture: None,
        })
    }

    fn make_targets(device: &wgpu::Device, post_bgl: &wgpu::BindGroupLayout, samp: &wgpu::Sampler, post_buf: &wgpu::Buffer, sw: u32, sh: u32, scale: f32) -> Targets {
        let w = ((sw as f32 * scale) as u32).max(16);
        let h = ((sh as f32 * scale) as u32).max(16);
        let rt = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let hdr_msaa = tex2d(device, w, h, HDR, wgpu::TextureUsages::RENDER_ATTACHMENT, MSAA).create_view(&Default::default());
        let depth = tex2d(device, w, h, DEPTH, wgpu::TextureUsages::RENDER_ATTACHMENT, MSAA).create_view(&Default::default());
        let hdr = tex2d(device, w, h, HDR, rt, 1).create_view(&Default::default());
        let mut bloom = Vec::new();
        let (mut bw, mut bh) = (w, h);
        for _ in 0..BLOOM_LEVELS {
            bw = (bw / 2).max(1);
            bh = (bh / 2).max(1);
            bloom.push((tex2d(device, bw, bh, HDR, rt, 1).create_view(&Default::default()), bw, bh));
        }
        let bg = |src: &wgpu::TextureView, extra: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: post_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(src) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(samp) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(extra) },
                    wgpu::BindGroupEntry { binding: 3, resource: post_buf.as_entire_binding() },
                ],
            })
        };
        // 降採樣：level i 從 i-1（第一級從 hdr）讀；第三個綁定只是佔位
        let mut down_bgs = Vec::new();
        for i in 0..BLOOM_LEVELS {
            let src = if i == 0 { &hdr } else { &bloom[i - 1].0 };
            let dummy = if i == 0 { &bloom[1].0 } else { &hdr };
            down_bgs.push(bg(src, dummy));
        }
        // 升採樣：level i 讀 i+1，加到 i
        let mut up_bgs = Vec::new();
        for i in 0..BLOOM_LEVELS - 1 {
            up_bgs.push(bg(&bloom[i + 1].0, &hdr));
        }
        let tonemap_bg = bg(&hdr, &bloom[0].0);
        Targets { hdr_msaa, depth, hdr, bloom, w, h, down_bgs, up_bgs, tonemap_bg }
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        self.config.width = w;
        self.config.height = h;
        self.surface.configure(&self.device, &self.config);
        self.targets = Self::make_targets(&self.device, &self.post_bgl, &self.lin_samp, &self.post_buf, w, h, self.render_scale);
    }

    pub fn set_scale(&mut self, s: f32) {
        self.render_scale = s;
        let (w, h) = (self.config.width, self.config.height);
        self.targets = Self::make_targets(&self.device, &self.post_bgl, &self.lin_samp, &self.post_buf, w, h, s);
    }

    /// 讀回完成時回傳 RGBA 像素
    pub fn take_capture(&mut self) -> Option<(u32, u32, Vec<u8>)> {
        let ready = self.capture.as_ref().map_or(false, |c| c.4.get());
        if !ready {
            return None;
        }
        let (buf, w, h, bpr, _) = self.capture.take().unwrap();
        let data = match buf.slice(..).get_mapped_range() { Ok(v) => v.to_vec(), Err(_) => return None };
        let bgra = matches!(self.config.format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            let row = &data[(y * bpr) as usize..(y * bpr + w * 4) as usize];
            for px in row.chunks(4) {
                if bgra {
                    out.extend_from_slice(&[px[2], px[1], px[0], 255]);
                } else {
                    out.extend_from_slice(&[px[0], px[1], px[2], 255]);
                }
            }
        }
        Some((w, h, out))
    }

    pub fn aspect(&self) -> f32 {
        self.config.width as f32 / self.config.height.max(1) as f32
    }

    pub fn render(&mut self, f: &FrameParams) {
        let cam = f.cam;
        // 陰影：以相機為中心的正交投影，中心對齊像素避免閃爍
        let extent = 160.0f32;
        let l = f.light_dir.normalize();
        let up = if l.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
        let light_view0 = glam::camera::rh::view::look_at_mat4(Vec3::ZERO, -l, up);
        let texel = 2.0 * extent / SHADOW_SIZE as f32;
        let c_ls = light_view0.transform_point3(cam);
        let snapped = Vec3::new((c_ls.x / texel).floor() * texel, (c_ls.y / texel).floor() * texel, c_ls.z);
        let center = light_view0.inverse().transform_point3(snapped);
        let light_view = glam::camera::rh::view::look_at_mat4(center + l * 800.0, center, up);
        let light_proj = glam::camera::rh::proj::directx::orthographic(-extent, extent, -extent, extent, 1.0, 1800.0);
        let shadow_mat = light_proj * light_view * Mat4::from_translation(cam);

        let g = Globals {
            view_proj: f.view_proj.to_cols_array_2d(),
            inv_view_proj: f.view_proj.inverse().to_cols_array_2d(),
            shadow_mat: shadow_mat.to_cols_array_2d(),
            cam_pos: [cam.x, cam.y, cam.z, f.time],
            light_dir: [l.x, l.y, l.z, f.night],
            light_color: [f.light_color.x, f.light_color.y, f.light_color.z, f.exposure],
            sun_dir: [f.sun_dir.x, f.sun_dir.y, f.sun_dir.z, 0.012 * f.night],
            ambient: [0.0, 0.0, 0.0, self.world_size],
            fog: [f.fog_density, 0.0016, f.cloud_cover, extent],
            screen: [self.targets.w as f32, self.targets.h as f32, 1.0 / self.targets.w as f32, 1.0 / self.targets.h as f32],
            grid: self.grid,
        };
        self.queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&g));
        let bloom_strength = 0.06 + 0.06 * f.night;
        self.queue.write_buffer(&self.post_buf, 0, bytemuck::cast_slice(&[f.exposure, bloom_strength, if self.manual_gamma { 1.0 } else { 0.0 }, f.time.fract()]));

        // 地形節點
        let planes = frustum_planes(f.view_proj);
        let mut nodes = std::mem::take(&mut self.node_scratch);
        nodes.clear();
        self.lod.select(cam, &planes, &self.minmax, None, &mut nodes);
        nodes.truncate(MAX_NODES);
        self.last_nodes = nodes.len();
        self.queue.write_buffer(&self.nodes_buf, 0, bytemuck::cast_slice(&nodes));
        let mut wnodes = std::mem::take(&mut self.water_scratch);
        wnodes.clear();
        self.lod.select(cam, &planes, &self.minmax, Some(&self.wet), &mut wnodes);
        wnodes.truncate(MAX_NODES);
        self.queue.write_buffer(&self.water_nodes_buf, 0, bytemuck::cast_slice(&wnodes));
        let parts = &f.parts[..f.parts.len().min(MAX_CAR_PARTS)];
        let n_box = parts.iter().filter(|p| p.color[3] != 2.0 && p.color[3] != 3.0).count();
        // parts 已按「方塊在前、圓柱在後」排列
        self.queue.write_buffer(&self.parts_buf, 0, bytemuck::cast_slice(parts));

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            other => {
                web_sys::console::warn_1(&format!("surface texture unavailable: {:?}", std::mem::discriminant(&other)).into());
                self.surface.configure(&self.device, &self.config);
                self.node_scratch = nodes;
                self.water_scratch = wnodes;
                return;
            }
        };
        let out_view = frame.texture.create_view(&Default::default());
        let mut enc = self.device.create_command_encoder(&Default::default());

        // 1. 天空查找表
        {
            let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sky lut"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &self.lut_view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            p.set_pipeline(&self.p_lut);
            p.set_bind_group(0, &self.globals_bg, &[]);
            p.draw(0..3, 0..1);
        }
        // 2. 陰影
        {
            let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &self.shadow_view, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }), stencil_ops: None }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            p.set_bind_group(0, &self.globals_bg, &[]);
            if self.building_count > 0 {
                p.set_pipeline(&self.p_building_shadow);
                p.set_vertex_buffer(0, self.cube_b.vb.slice(..));
                p.set_vertex_buffer(1, self.buildings.slice(..));
                p.set_index_buffer(self.cube_b.ib.slice(..), wgpu::IndexFormat::Uint16);
                p.draw_indexed(0..self.cube_b.count, 0, 0..self.building_count);
            }
            p.set_pipeline(&self.p_car_shadow);
            p.set_vertex_buffer(0, self.cube_c.vb.slice(..));
            p.set_vertex_buffer(1, self.parts_buf.slice(..));
            p.set_index_buffer(self.cube_c.ib.slice(..), wgpu::IndexFormat::Uint16);
            p.draw_indexed(0..self.cube_c.count, 0, 0..n_box as u32);
        }
        // 3. 主場景
        {
            let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.targets.hdr_msaa,
                    depth_slice: None,
                    resolve_target: Some(&self.targets.hdr),
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Discard },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &self.targets.depth, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Discard }), stencil_ops: None }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            p.set_bind_group(0, &self.scene_bg, &[]);
            // 車輛（最近，先畫利於提前深度剔除）
            p.set_pipeline(&self.p_car);
            p.set_vertex_buffer(0, self.cube_c.vb.slice(..));
            p.set_vertex_buffer(1, self.parts_buf.slice(..));
            p.set_index_buffer(self.cube_c.ib.slice(..), wgpu::IndexFormat::Uint16);
            p.draw_indexed(0..self.cube_c.count, 0, 0..n_box as u32);
            if parts.len() > n_box {
                p.set_vertex_buffer(0, self.cyl.vb.slice(..));
                p.set_index_buffer(self.cyl.ib.slice(..), wgpu::IndexFormat::Uint16);
                p.draw_indexed(0..self.cyl.count, 0, n_box as u32..parts.len() as u32);
            }
            // 道路
            p.set_pipeline(&self.p_road);
            p.set_vertex_buffer(0, self.roads.vb.slice(..));
            p.set_index_buffer(self.roads.ib.slice(..), wgpu::IndexFormat::Uint32);
            p.draw_indexed(0..self.roads.count, 0, 0..1);
            // 建築
            if self.building_count > 0 {
                p.set_pipeline(&self.p_building);
                p.set_vertex_buffer(0, self.cube_b.vb.slice(..));
                p.set_vertex_buffer(1, self.buildings.slice(..));
                p.set_index_buffer(self.cube_b.ib.slice(..), wgpu::IndexFormat::Uint16);
                p.draw_indexed(0..self.cube_b.count, 0, 0..self.building_count);
            }
            // 地形
            p.set_pipeline(&self.p_terrain);
            p.set_vertex_buffer(0, self.patch.vb.slice(..));
            p.set_vertex_buffer(1, self.nodes_buf.slice(..));
            p.set_index_buffer(self.patch.ib.slice(..), wgpu::IndexFormat::Uint16);
            p.draw_indexed(0..self.patch.count, 0, 0..nodes.len() as u32);
            // 水面
            if !wnodes.is_empty() {
                p.set_pipeline(&self.p_water);
                p.set_vertex_buffer(1, self.water_nodes_buf.slice(..));
                p.draw_indexed(0..self.patch.count, 0, 0..wnodes.len() as u32);
            }
            p.set_pipeline(&self.p_ocean);
            p.draw_indexed(0..self.patch.count, 0, 0..1);
            // 天空最後畫
            p.set_pipeline(&self.p_sky);
            p.set_bind_group(0, &self.sky_bg, &[]);
            p.draw(0..3, 0..1);
        }
        // 4. 泛光
        for i in 0..BLOOM_LEVELS {
            let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bloom down"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &self.targets.bloom[i].0, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            p.set_pipeline(if i == 0 { &self.p_down_first } else { &self.p_down });
            p.set_bind_group(0, &self.targets.down_bgs[i], &[]);
            p.draw(0..3, 0..1);
        }
        for i in (0..BLOOM_LEVELS - 1).rev() {
            let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bloom up"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &self.targets.bloom[i].0, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            p.set_pipeline(&self.p_up);
            p.set_bind_group(0, &self.targets.up_bgs[i], &[]);
            p.draw(0..3, 0..1);
        }
        // 5. 色調映射到畫面
        {
            let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("tonemap"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &out_view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            p.set_pipeline(&self.p_tonemap);
            p.set_bind_group(0, &self.targets.tonemap_bg, &[]);
            p.draw(0..3, 0..1);
        }
        let mut cap = None;
        if self.capture_request {
            self.capture_request = false;
            let (w, h) = (frame.texture.width(), frame.texture.height());
            let bpr = (w * 4).div_ceil(256) * 256;
            let buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("capture"), size: (bpr * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
            enc.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo { texture: &frame.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(h) } },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            cap = Some((buf, w, h, bpr));
        }
        self.queue.submit([enc.finish()]);
        self.queue.present(frame);
        if let Some((buf, w, h, bpr)) = cap {
            let ready = std::rc::Rc::new(std::cell::Cell::new(false));
            let r2 = ready.clone();
            buf.slice(..).map_async(wgpu::MapMode::Read, move |res| r2.set(res.is_ok()));
            self.capture = Some((buf, w, h, bpr, ready));
        }
        self.node_scratch = nodes;
        self.water_scratch = wnodes;
    }
}
