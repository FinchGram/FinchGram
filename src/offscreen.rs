//! A window that is never on a screen, drawn as the app's own window is: by Slint's Skia renderer
//! over Metal (docs/skia.md), but into a texture, which is read back as pixels. For the checks that
//! have to see what Skia draws and need no display: the pictures of every page
//! (`cargo test skia_pictures -- --ignored`, src/screenshots.rs) and the video frames
//! (`cargo test video_frames -- --ignored`, src/player/frames.rs).
//!
//! It needs a GPU. The windows of one test share one device, as the app's windows do.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::rc::{Rc, Weak};
use std::task::{Context, Poll, Waker};

use slint::platform::skia_renderer::SkiaWGPU30Renderer;
use slint::platform::{WindowAdapter, WindowEvent};
use slint::wgpu_30::wgpu;

/// The Metal device the windows are drawn with.
pub struct Graphics {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl Graphics {
    pub fn new() -> Rc<Graphics> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            flags: wgpu::InstanceFlags::from_build_config(),
            backend_options: wgpu::BackendOptions::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            display: None,
        });
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
        .expect("a Metal adapter");
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: None,
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::default(),
        }))
        .expect("a Metal device");
        Rc::new(Graphics { instance, adapter, device, queue })
    }
}

/// wgpu's futures are ready at once on a Mac: no executor needed.
fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
        std::thread::yield_now();
    }
}

/// One window: what a test's platform hands Slint as the window's adapter.
pub struct Window {
    window: slint::Window,
    renderer: SkiaWGPU30Renderer,
    graphics: Rc<Graphics>,
    size: Cell<slint::PhysicalSize>,
    /// What the window is drawn into, of the window's size.
    picture: RefCell<Option<wgpu::Texture>>,
}

impl WindowAdapter for Window {
    fn window(&self) -> &slint::Window {
        &self.window
    }

    fn size(&self) -> slint::PhysicalSize {
        self.size.get()
    }

    fn renderer(&self) -> &dyn slint::platform::Renderer {
        &self.renderer
    }
}

impl Window {
    pub fn new(graphics: &Rc<Graphics>) -> Rc<Window> {
        let renderer = SkiaWGPU30Renderer::new(graphics.instance.clone(), graphics.adapter.clone(), graphics.device.clone(), graphics.queue.clone())
            .expect("Slint's Skia renderer");
        Rc::new_cyclic(|weak: &Weak<Window>| Window {
            window: slint::Window::new(weak.clone()),
            renderer,
            graphics: graphics.clone(),
            size: Cell::new(slint::PhysicalSize::new(1, 1)),
            picture: RefCell::new(None),
        })
    }

    /// So many pixels, and as many points: the scale is 1.
    pub fn set_size(&self, size: slint::PhysicalSize) {
        self.size.set(size);
        self.picture.borrow_mut().take();
        self.window.dispatch_event(WindowEvent::Resized { size: slint::LogicalSize::new(size.width as f32, size.height as f32) });
    }

    pub fn dispatch_event(&self, event: WindowEvent) {
        self.window.dispatch_event(event);
    }

    /// Draw the window as it is now: its pixels, row by row from the top, as red, green, blue, alpha.
    pub fn draw(&self) -> Vec<u8> {
        let (width, height) = (self.size.get().width, self.size.get().height);
        let (device, queue) = (&self.graphics.device, &self.graphics.queue);
        let mut picture = self.picture.borrow_mut();
        let picture = picture.get_or_insert_with(|| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("the window"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        });
        // Skia draws through Metal behind wgpu's back, and wgpu clears a texture it believes was
        // never written before it copies from it: a clear of wgpu's own first.
        let view = picture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        queue.submit(Some(encoder.finish()));
        self.renderer.render_to_texture(picture).expect("the window drawn");

        // A row in the buffer is a multiple of 256 bytes long, wgpu's rule; the rest of it is left out.
        let row = width * 4;
        let padded = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(padded * height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: picture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(height) } },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        queue.submit(Some(encoder.finish()));
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |result| result.expect("the picture mapped"));
        device.poll(wgpu::PollType::wait_indefinitely()).expect("the picture read");
        let mapped = slice.get_mapped_range().expect("the picture's bytes");
        let mut pixels = Vec::with_capacity((row * height) as usize);
        for line in mapped.chunks(padded as usize) {
            pixels.extend_from_slice(&line[..row as usize]);
        }
        drop(mapped);
        buffer.unmap();
        pixels
    }
}

/// Every page in every theme, light and dark, as Skia draws it over Metal: the pictures of
/// src/screenshots.rs, into target/screenshots-skia/.
#[test]
#[ignore = "needs a GPU and takes over Slint's platform; run with: cargo test skia_pictures -- --ignored"]
fn skia_pictures() {
    crate::screenshots::draw_with_skia();
}
