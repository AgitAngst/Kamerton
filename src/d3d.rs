// Рисовальщик egui на Direct3D 11. Голое устройство D3D11 стоит ~4 МБ своей памяти против ~43 МБ контекста
// OpenGL и сотен МБ wgpu (замер 22.10.2026) — для маленького окна это главное.
//
// Цвета egui — sRGB с умноженной альфой; рисуем в обычный (не sRGB) буфер и смешиваем в гамме, как
// egui_glow: текстуры R8G8B8A8_UNORM, смешивание ONE / INV_SRC_ALPHA.

use std::collections::HashMap;
use std::ffi::c_void;

use eframe::egui::{self, ClippedPrimitive, Color32, ColorImage, TextureId, epaint::Primitive};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, ID3DBlob,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R32_UINT,
    DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1,
    DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIFactory2,
    IDXGISwapChain1,
};
use windows::core::{PCSTR, s};

const SHADER: &str = r#"
cbuffer Screen : register(b0) { float2 size; float2 pad; };
struct VsIn { float2 pos : POSITION; float2 uv : TEXCOORD; float4 color : COLOR; };
struct PsIn { float4 pos : SV_POSITION; float2 uv : TEXCOORD; float4 color : COLOR; };
PsIn vs(VsIn i) {
    PsIn o;
    o.pos = float4(2.0 * i.pos.x / size.x - 1.0, 1.0 - 2.0 * i.pos.y / size.y, 0.0, 1.0);
    o.uv = i.uv;
    o.color = i.color;
    return o;
}
Texture2D tex : register(t0);
SamplerState smp : register(s0);
float4 ps(PsIn i) : SV_TARGET { return i.color * tex.Sample(smp, i.uv); }
"#;

struct Texture {
    tex: ID3D11Texture2D,
    view: ID3D11ShaderResourceView,
    linear: bool,
}

pub struct Painter {
    device: ID3D11Device,
    ctx: ID3D11DeviceContext,
    swap: IDXGISwapChain1,
    target: Option<ID3D11RenderTargetView>,
    size: [u32; 2],
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    layout: ID3D11InputLayout,
    blend: ID3D11BlendState,
    raster: ID3D11RasterizerState,
    linear: ID3D11SamplerState,
    nearest: ID3D11SamplerState,
    screen: ID3D11Buffer,
    vertices: Option<(ID3D11Buffer, usize)>,
    indices: Option<(ID3D11Buffer, usize)>,
    textures: HashMap<TextureId, Texture>,
}

fn blob_bytes(b: &ID3DBlob) -> &[u8] {
    unsafe { std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()) }
}

fn compile(entry: PCSTR, target: PCSTR) -> Result<ID3DBlob, String> {
    let mut code = None;
    let mut errors = None;
    let r = unsafe {
        D3DCompile(
            SHADER.as_ptr() as *const c_void,
            SHADER.len(),
            None,
            None,
            None,
            entry,
            target,
            0,
            0,
            &mut code,
            Some(&mut errors),
        )
    };
    if let Err(e) = r {
        let msg = errors.as_ref().map(|b| String::from_utf8_lossy(blob_bytes(b)).to_string()).unwrap_or_default();
        return Err(format!("shader: {e} {msg}"));
    }
    code.ok_or_else(|| "shader: no code".into())
}

fn e(err: windows::core::Error) -> String {
    err.message()
}

impl Painter {
    pub fn new(hwnd: isize) -> Result<Painter, String> {
        unsafe {
            let mut device = None;
            let mut ctx = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                Default::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut ctx),
            )
            .map_err(e)?;
            let device: ID3D11Device = device.ok_or("no device")?;
            let ctx: ID3D11DeviceContext = ctx.ok_or("no context")?;

            let factory: IDXGIFactory2 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)).map_err(e)?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                ..Default::default()
            };
            let swap = factory.CreateSwapChainForHwnd(&device, HWND(hwnd as *mut _), &desc, None, None).map_err(e)?;

            let vs_code = compile(s!("vs"), s!("vs_4_0"))?;
            let ps_code = compile(s!("ps"), s!("ps_4_0"))?;
            let mut vs = None;
            device.CreateVertexShader(blob_bytes(&vs_code), None, Some(&mut vs)).map_err(e)?;
            let mut ps = None;
            device.CreatePixelShader(blob_bytes(&ps_code), None, Some(&mut ps)).map_err(e)?;
            let elements = [
                input(s!("POSITION"), DXGI_FORMAT_R32G32_FLOAT, 0),
                input(s!("TEXCOORD"), DXGI_FORMAT_R32G32_FLOAT, 8),
                input(s!("COLOR"), DXGI_FORMAT_R8G8B8A8_UNORM, 16),
            ];
            let mut layout = None;
            device.CreateInputLayout(&elements, blob_bytes(&vs_code), Some(&mut layout)).map_err(e)?;

            let mut blend_desc = D3D11_BLEND_DESC::default();
            blend_desc.RenderTarget[0] = D3D11_RENDER_TARGET_BLEND_DESC {
                BlendEnable: true.into(),
                SrcBlend: D3D11_BLEND_ONE,
                DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
                BlendOp: D3D11_BLEND_OP_ADD,
                SrcBlendAlpha: D3D11_BLEND_INV_DEST_ALPHA,
                DestBlendAlpha: D3D11_BLEND_ONE,
                BlendOpAlpha: D3D11_BLEND_OP_ADD,
                RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
            };
            let mut blend = None;
            device.CreateBlendState(&blend_desc, Some(&mut blend)).map_err(e)?;
            let raster_desc = D3D11_RASTERIZER_DESC {
                FillMode: D3D11_FILL_SOLID,
                CullMode: D3D11_CULL_NONE,
                ScissorEnable: true.into(),
                DepthClipEnable: true.into(),
                ..Default::default()
            };
            let mut raster = None;
            device.CreateRasterizerState(&raster_desc, Some(&mut raster)).map_err(e)?;
            let sampler = |filter| {
                let d = D3D11_SAMPLER_DESC {
                    Filter: filter,
                    AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                    AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                    AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                    ComparisonFunc: D3D11_COMPARISON_NEVER,
                    MaxLOD: f32::MAX,
                    ..Default::default()
                };
                let mut s = None;
                device.CreateSamplerState(&d, Some(&mut s)).map(|_| s.unwrap())
            };
            let linear = sampler(D3D11_FILTER_MIN_MAG_MIP_LINEAR).map_err(e)?;
            let nearest = sampler(D3D11_FILTER_MIN_MAG_MIP_POINT).map_err(e)?;
            let cb = D3D11_BUFFER_DESC {
                ByteWidth: 16,
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                ..Default::default()
            };
            let mut screen = None;
            device.CreateBuffer(&cb, None, Some(&mut screen)).map_err(e)?;

            Ok(Painter {
                device,
                ctx,
                swap,
                target: None,
                size: [0, 0],
                vs: vs.ok_or("vs")?,
                ps: ps.ok_or("ps")?,
                layout: layout.ok_or("layout")?,
                blend: blend.ok_or("blend")?,
                raster: raster.ok_or("raster")?,
                linear,
                nearest,
                screen: screen.ok_or("cbuffer")?,
                vertices: None,
                indices: None,
                textures: HashMap::new(),
            })
        }
    }

    fn ensure_target(&mut self, size: [u32; 2]) -> Result<(), String> {
        if self.target.is_some() && self.size == size {
            return Ok(());
        }
        unsafe {
            self.target = None;
            self.ctx.OMSetRenderTargets(None, None);
            // Цепочка создана под размер окна в момент создания — подгоняем и на первом кадре.
            self.swap.ResizeBuffers(0, size[0], size[1], DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0)).map_err(e)?;
            let back: ID3D11Texture2D = self.swap.GetBuffer(0).map_err(e)?;
            let mut rtv = None;
            self.device.CreateRenderTargetView(&back, None, Some(&mut rtv)).map_err(e)?;
            self.target = rtv;
            self.size = size;
        }
        Ok(())
    }

    fn set_texture(&mut self, id: TextureId, delta: &egui::epaint::ImageDelta) -> Result<(), String> {
        let egui::ImageData::Color(image) = &delta.image;
        let [w, h] = image.size;
        let pixels: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
        let linear = delta.options.magnification == egui::TextureFilter::Linear;
        unsafe {
            if let Some(pos) = delta.pos {
                let Some(t) = self.textures.get(&id) else { return Ok(()) };
                let b = D3D11_BOX {
                    left: pos[0] as u32,
                    top: pos[1] as u32,
                    front: 0,
                    right: (pos[0] + w) as u32,
                    bottom: (pos[1] + h) as u32,
                    back: 1,
                };
                self.ctx.UpdateSubresource(&t.tex, 0, Some(&b), pixels.as_ptr() as *const c_void, (w * 4) as u32, 0);
                return Ok(());
            }
            let desc = D3D11_TEXTURE2D_DESC {
                Width: w as u32,
                Height: h as u32,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                ..Default::default()
            };
            let data = D3D11_SUBRESOURCE_DATA {
                pSysMem: pixels.as_ptr() as *const c_void,
                SysMemPitch: (w * 4) as u32,
                SysMemSlicePitch: 0,
            };
            let mut tex = None;
            self.device.CreateTexture2D(&desc, Some(&data), Some(&mut tex)).map_err(e)?;
            let tex: ID3D11Texture2D = tex.ok_or("texture")?;
            let mut view = None;
            self.device.CreateShaderResourceView(&tex, None, Some(&mut view)).map_err(e)?;
            self.textures.insert(id, Texture { tex, view: view.ok_or("view")?, linear });
        }
        Ok(())
    }

    fn upload<T: Copy>(
        device: &ID3D11Device,
        ctx: &ID3D11DeviceContext,
        slot: &mut Option<(ID3D11Buffer, usize)>,
        data: &[T],
        bind: D3D11_BIND_FLAG,
    ) -> Result<(), String> {
        let bytes = std::mem::size_of_val(data);
        if bytes == 0 {
            return Ok(());
        }
        unsafe {
            if slot.as_ref().is_none_or(|(_, cap)| *cap < bytes) {
                let cap = bytes.next_power_of_two().max(4096);
                let desc = D3D11_BUFFER_DESC {
                    ByteWidth: cap as u32,
                    Usage: D3D11_USAGE_DYNAMIC,
                    BindFlags: bind.0 as u32,
                    CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                    ..Default::default()
                };
                let mut b = None;
                device.CreateBuffer(&desc, None, Some(&mut b)).map_err(e)?;
                *slot = Some((b.ok_or("buffer")?, cap));
            }
            let (buf, _) = slot.as_ref().unwrap();
            let mut map = D3D11_MAPPED_SUBRESOURCE::default();
            ctx.Map(buf, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut map)).map_err(e)?;
            std::ptr::copy_nonoverlapping(data.as_ptr() as *const u8, map.pData as *mut u8, bytes);
            ctx.Unmap(buf, 0);
        }
        Ok(())
    }

    /// Нарисовать кадр egui и показать его. `size` — пиксели окна, `ppp` — пикселей на точку.
    pub fn paint(
        &mut self,
        size: [u32; 2],
        ppp: f32,
        clear: Color32,
        prims: &[ClippedPrimitive],
        textures: &egui::TexturesDelta,
        capture: bool,
    ) -> Result<Option<ColorImage>, String> {
        for (id, deltas) in &textures.set {
            for delta in deltas {
                self.set_texture(*id, delta)?;
            }
        }
        if size[0] == 0 || size[1] == 0 {
            for id in &textures.free {
                self.textures.remove(id);
            }
            return Ok(None);
        }
        self.ensure_target(size)?;
        // Все сетки кадра — в одни буферы, рисуем кусками.
        let mut vertices: Vec<egui::epaint::Vertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut draws = Vec::new();
        for p in prims {
            if let Primitive::Mesh(m) = &p.primitive {
                if m.indices.is_empty() {
                    continue;
                }
                let base = vertices.len() as u32;
                let first = indices.len() as u32;
                vertices.extend_from_slice(&m.vertices);
                indices.extend(m.indices.iter().map(|i| i + base));
                draws.push((p.clip_rect, m.texture_id, first, m.indices.len() as u32));
            }
        }
        Self::upload(&self.device, &self.ctx, &mut self.vertices, &vertices, D3D11_BIND_VERTEX_BUFFER)?;
        Self::upload(&self.device, &self.ctx, &mut self.indices, &indices, D3D11_BIND_INDEX_BUFFER)?;
        unsafe {
            let ctx = &self.ctx;
            let target = self.target.clone();
            let c = clear.to_normalized_gamma_f32();
            ctx.ClearRenderTargetView(target.as_ref().unwrap(), &c);
            ctx.OMSetRenderTargets(Some(&[target]), None);
            ctx.OMSetBlendState(&self.blend, Some(&[0.0; 4]), 0xFFFF_FFFF);
            ctx.RSSetState(&self.raster);
            ctx.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: size[0] as f32,
                Height: size[1] as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]));
            let points = [size[0] as f32 / ppp, size[1] as f32 / ppp, 0.0, 0.0];
            let mut map = D3D11_MAPPED_SUBRESOURCE::default();
            ctx.Map(&self.screen, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut map)).map_err(e)?;
            std::ptr::copy_nonoverlapping(points.as_ptr() as *const u8, map.pData as *mut u8, 16);
            ctx.Unmap(&self.screen, 0);
            ctx.IASetInputLayout(&self.layout);
            ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            if let (Some((vb, _)), Some((ib, _))) = (&self.vertices, &self.indices) {
                let stride = size_of::<egui::epaint::Vertex>() as u32;
                ctx.IASetVertexBuffers(0, 1, Some(&Some(vb.clone())), Some(&stride), Some(&0));
                ctx.IASetIndexBuffer(ib, DXGI_FORMAT_R32_UINT, 0);
            }
            ctx.VSSetShader(&self.vs, None);
            ctx.VSSetConstantBuffers(0, Some(&[Some(self.screen.clone())]));
            ctx.PSSetShader(&self.ps, None);
            for (clip, tex, first, count) in draws {
                let Some(t) = self.textures.get(&tex) else { continue };
                let l = (clip.min.x * ppp).round().clamp(0.0, size[0] as f32) as i32;
                let top = (clip.min.y * ppp).round().clamp(0.0, size[1] as f32) as i32;
                let r = (clip.max.x * ppp).round().clamp(0.0, size[0] as f32) as i32;
                let b = (clip.max.y * ppp).round().clamp(0.0, size[1] as f32) as i32;
                if r <= l || b <= top {
                    continue;
                }
                ctx.RSSetScissorRects(Some(&[windows::Win32::Foundation::RECT { left: l, top, right: r, bottom: b }]));
                ctx.PSSetShaderResources(0, Some(&[Some(t.view.clone())]));
                let sampler = if t.linear { &self.linear } else { &self.nearest };
                ctx.PSSetSamplers(0, Some(&[Some(sampler.clone())]));
                ctx.DrawIndexed(count, first, 0);
            }
            // Снимок — до Present: после него у flip-цепочки задний буфер уже не наш.
            let shot = if capture { Some(self.read_back()?) } else { None };
            self.swap.Present(1, DXGI_PRESENT(0)).ok().map_err(e)?;
            for id in &textures.free {
                self.textures.remove(id);
            }
            Ok(shot)
        }
    }

    /// Копия заднего буфера в RGBA.
    fn read_back(&self) -> Result<ColorImage, String> {
        unsafe {
            let back: ID3D11Texture2D = self.swap.GetBuffer(0).map_err(e)?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            back.GetDesc(&mut desc);
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            desc.MiscFlags = 0;
            let mut staging = None;
            self.device.CreateTexture2D(&desc, None, Some(&mut staging)).map_err(e)?;
            let staging: ID3D11Texture2D = staging.ok_or("staging")?;
            self.ctx.CopyResource(&staging, &back);
            let mut map = D3D11_MAPPED_SUBRESOURCE::default();
            self.ctx.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut map)).map_err(e)?;
            let (w, h) = (desc.Width as usize, desc.Height as usize);
            let mut pixels = Vec::with_capacity(w * h);
            for y in 0..h {
                let row = std::slice::from_raw_parts((map.pData as *const u8).add(y * map.RowPitch as usize), w * 4);
                for px in row.as_chunks::<4>().0 {
                    pixels.push(Color32::from_rgba_premultiplied(px[2], px[1], px[0], 255));
                }
            }
            self.ctx.Unmap(&staging, 0);
            Ok(ColorImage::new([w, h], pixels))
        }
    }
}

fn input(name: PCSTR, format: DXGI_FORMAT, offset: u32) -> D3D11_INPUT_ELEMENT_DESC {
    D3D11_INPUT_ELEMENT_DESC {
        SemanticName: name,
        SemanticIndex: 0,
        Format: format,
        InputSlot: 0,
        AlignedByteOffset: offset,
        InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
        InstanceDataStepRate: 0,
    }
}
