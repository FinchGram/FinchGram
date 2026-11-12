//! A video frame's way from mpv to the window on macOS (docs/skia.md), for src/player/.
//!
//! libmpv's render API draws through OpenGL or into plain memory, nothing else, and the window is
//! drawn over Metal. So mpv gets an OpenGL context of its own with no window (CGL), and draws into
//! an IOSurface: OpenGL sees it as a texture with a framebuffer around it, Metal sees the same
//! memory as a texture, which wgpu wraps and Slint takes as an image. No pixel is copied on the
//! way, and the two never share a context, so nothing of OpenGL's state has to be saved or put back
//! around mpv: mpv's drawing is flushed, then the window's reads.
//!
//! The handful of OpenGL calls FinchGram itself makes (a texture, a framebuffer, a flush) are
//! declared here by hand; mpv finds the functions it needs through `lookup`.

use std::ffi::{c_char, c_int, c_void};
use std::ptr;

use objc2_core_foundation::{CFDictionary, CFMutableDictionary, CFNumber, CFString, CFType};
use objc2_io_surface::{IOSurfaceRef, kIOSurfaceBytesPerElement, kIOSurfaceHeight, kIOSurfacePixelFormat, kIOSurfaceWidth};
use objc2_metal::{
    MTLDevice, MTLPixelFormat, MTLStorageMode, MTLTextureDescriptor, MTLTextureSwizzle, MTLTextureSwizzleChannels, MTLTextureType,
    MTLTextureUsage,
};
use slint::wgpu_30::wgpu;

// ---- OpenGL, in a context of a player's own -------------------------------------------------------

type CGLContextObj = *mut c_void;
type CGLPixelFormatObj = *mut c_void;

/// CGL's pixel format attributes (OpenGL/CGLTypes.h): a hardware renderer, the 3.2 core profile
/// (the system gives its newest, 4.1), eight bits a channel.
const PFA_COLOR_SIZE: u32 = 8;
const PFA_ALPHA_SIZE: u32 = 11;
const PFA_ACCELERATED: u32 = 73;
const PFA_OPENGL_PROFILE: u32 = 99;
const PROFILE_3_2_CORE: u32 = 0x3200;

const TEXTURE_RECTANGLE: u32 = 0x84F5;
const RGBA: u32 = 0x1908;
const RGBA8: u32 = 0x8058;
const BGRA: u32 = 0x80E1;
const UNSIGNED_INT_8_8_8_8_REV: u32 = 0x8367;
const FRAMEBUFFER: u32 = 0x8D40;
const COLOR_ATTACHMENT0: u32 = 0x8CE0;
const FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;

#[link(name = "OpenGL", kind = "framework")]
unsafe extern "C" {
    fn CGLChoosePixelFormat(attributes: *const u32, format: *mut CGLPixelFormatObj, count: *mut c_int) -> c_int;
    fn CGLDestroyPixelFormat(format: CGLPixelFormatObj) -> c_int;
    fn CGLCreateContext(format: CGLPixelFormatObj, share: CGLContextObj, context: *mut CGLContextObj) -> c_int;
    fn CGLReleaseContext(context: CGLContextObj);
    fn CGLSetCurrentContext(context: CGLContextObj) -> c_int;
    fn CGLGetCurrentContext() -> CGLContextObj;
    /// Make the texture bound to `target` draw into, and read from, `surface` (an IOSurfaceRef).
    fn CGLTexImageIOSurface2D(
        context: CGLContextObj,
        target: u32,
        internal_format: u32,
        width: c_int,
        height: c_int,
        format: u32,
        kind: u32,
        surface: *const c_void,
        plane: u32,
    ) -> c_int;

    fn glGenTextures(count: c_int, textures: *mut u32);
    fn glDeleteTextures(count: c_int, textures: *const u32);
    fn glBindTexture(target: u32, texture: u32);
    fn glGenFramebuffers(count: c_int, framebuffers: *mut u32);
    fn glDeleteFramebuffers(count: c_int, framebuffers: *const u32);
    fn glBindFramebuffer(target: u32, framebuffer: u32);
    fn glFramebufferTexture2D(target: u32, attachment: u32, texture_target: u32, texture: u32, level: c_int);
    fn glCheckFramebufferStatus(target: u32) -> u32;
    fn glFlush();
}

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

/// dlfcn.h's RTLD_DEFAULT on macOS: every library the program has loaded.
const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;

/// An OpenGL function by its name, for mpv (mpv_opengl_init_params.get_proc_address): looked up
/// among the libraries the program has loaded, of which the system's OpenGL is one (linked above).
pub unsafe extern "C" fn lookup(_context: *mut c_void, name: *const c_char) -> *mut c_void {
    // SAFETY: a name from mpv, valid for the call.
    unsafe { dlsym(RTLD_DEFAULT, name) }
}

/// What mpv is told its framebuffer holds (mpv_opengl_fbo.internal_format): eight bits a channel.
pub const FRAMEBUFFER_FORMAT: c_int = RGBA8 as c_int;

/// Hand what OpenGL has drawn in the entered context over to the GPU: after it, what the window
/// draws reads the frame mpv has just drawn.
pub fn flush() {
    // SAFETY: a call with no arguments, in whichever context is entered.
    unsafe { glFlush() };
}

/// An OpenGL context with no window, for one player's mpv.
pub struct Context {
    raw: CGLContextObj,
}

impl Context {
    pub fn new() -> Result<Context, String> {
        let attributes = [PFA_OPENGL_PROFILE, PROFILE_3_2_CORE, PFA_ACCELERATED, PFA_COLOR_SIZE, 24, PFA_ALPHA_SIZE, 8, 0];
        let mut format: CGLPixelFormatObj = ptr::null_mut();
        let mut count: c_int = 0;
        // SAFETY: a zero-terminated list of attributes; the format is freed here, the context by Drop.
        unsafe {
            let status = CGLChoosePixelFormat(attributes.as_ptr(), &mut format, &mut count);
            if status != 0 || format.is_null() {
                return Err(format!("the system has no OpenGL to draw video with (CGL error {status})"));
            }
            let mut raw: CGLContextObj = ptr::null_mut();
            let status = CGLCreateContext(format, ptr::null_mut(), &mut raw);
            CGLDestroyPixelFormat(format);
            if status != 0 || raw.is_null() {
                return Err(format!("the system made no OpenGL context to draw video in (CGL error {status})"));
            }
            Ok(Context { raw })
        }
    }

    /// Make this the thread's OpenGL context until the guard goes; the one before, usually none,
    /// comes back then.
    pub fn enter(&self) -> Entered {
        // SAFETY: our own context, on the thread that made it (the UI thread).
        unsafe {
            let before = CGLGetCurrentContext();
            CGLSetCurrentContext(self.raw);
            Entered { before }
        }
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        // SAFETY: released once.
        unsafe { CGLReleaseContext(self.raw) };
    }
}

pub struct Entered {
    before: CGLContextObj,
}

impl Drop for Entered {
    fn drop(&mut self) {
        // SAFETY: the context that was current before, or none.
        unsafe { CGLSetCurrentContext(self.before) };
    }
}

// ---- the surface OpenGL and Metal share -----------------------------------------------------------

/// 'BGRA': the only order OpenGL takes for a surface of four bytes a pixel (CGLTexImageIOSurface2D
/// refuses GL_RGBA with error 10008; measured 2026-11-11).
const SURFACE_BGRA: i32 = i32::from_be_bytes(*b"BGRA");

/// What mpv draws into and the page shows.
pub struct Surface {
    pub width: i32,
    pub height: i32,
    /// OpenGL's texture that is the surface, and the framebuffer mpv draws into.
    texture: u32,
    pub framebuffer: u32,
    /// The surface as Slint takes it. It belongs to the device of the window's renderer, and keeps
    /// the surface's memory for as long as a page still shows it.
    pub shown: wgpu::Texture,
}

impl Surface {
    /// A surface of the video's size; with `context` entered. `device` is the one the window is
    /// drawn with.
    pub fn new(context: &Context, device: &wgpu::Device, width: i32, height: i32) -> Result<Surface, String> {
        let properties = CFMutableDictionary::<CFString, CFType>::empty();
        // SAFETY: IOSurface's own keys, with numbers; the dictionary keeps its own references.
        let surface = unsafe {
            properties.set(kIOSurfaceWidth, AsRef::<CFType>::as_ref(&*CFNumber::new_i32(width)));
            properties.set(kIOSurfaceHeight, AsRef::<CFType>::as_ref(&*CFNumber::new_i32(height)));
            properties.set(kIOSurfaceBytesPerElement, AsRef::<CFType>::as_ref(&*CFNumber::new_i32(4)));
            properties.set(kIOSurfacePixelFormat, AsRef::<CFType>::as_ref(&*CFNumber::new_i32(SURFACE_BGRA)));
            IOSurfaceRef::new(AsRef::<CFDictionary>::as_ref(properties.as_opaque()))
        }
        .ok_or("the system made no surface")?;

        // Metal's side first: when it fails, nothing of OpenGL's is left behind. Slint takes RGBA
        // textures only, and OpenGL writes blue first: the texture hands red and blue out swapped
        // back (a swizzle), which costs nothing. Metal lets no texture with a swizzle be a render
        // target, and this one need not be: it is only ever read.
        let (wide, high) = (width as u32, height as u32);
        // SAFETY: the device is wgpu's own Metal device; the descriptor describes the surface.
        let metal_texture = unsafe {
            let metal = device.as_hal::<wgpu::hal::api::Metal>().ok_or("the window is not drawn with Metal")?;
            let descriptor =
                MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(MTLPixelFormat::RGBA8Unorm, wide as usize, high as usize, false);
            descriptor.setUsage(MTLTextureUsage::ShaderRead);
            descriptor.setStorageMode(MTLStorageMode::Shared);
            descriptor.setSwizzle(MTLTextureSwizzleChannels {
                red: MTLTextureSwizzle::Blue,
                green: MTLTextureSwizzle::Green,
                blue: MTLTextureSwizzle::Red,
                alpha: MTLTextureSwizzle::Alpha,
            });
            metal.raw_device().newTextureWithDescriptor_iosurface_plane(&descriptor, &surface, 0)
        }
        .ok_or("Metal made no texture over the surface")?;
        // SAFETY: the texture is this device's and is what the descriptor says, but for
        // RENDER_ATTACHMENT, which Slint asks of every texture it is given (Image::try_from) and
        // never uses: it only reads (i-slint-renderer-skia, import_metal_texture).
        let shown = unsafe {
            let wrapped = wgpu::hal::metal::Device::texture_from_raw(
                metal_texture,
                wgpu::TextureFormat::Rgba8Unorm,
                MTLTextureType::Type2D,
                1,
                1,
                wgpu::hal::CopyExtent { width: wide, height: high, depth: 1 },
                None,
            );
            device.create_texture_from_hal::<wgpu::hal::api::Metal>(
                wrapped,
                &wgpu::TextureDescriptor {
                    label: Some("video frame"),
                    size: wgpu::Extent3d { width: wide, height: high, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                },
                wgpu::TextureUses::RESOURCE,
            )
        };

        // OpenGL's side: a texture that is the surface (it keeps the surface while it lives), in a
        // framebuffer for mpv to draw into.
        let (mut texture, mut framebuffer) = (0, 0);
        // SAFETY: plain OpenGL calls in the player's own context, which the caller entered.
        unsafe {
            glGenTextures(1, &mut texture);
            glBindTexture(TEXTURE_RECTANGLE, texture);
            let status = CGLTexImageIOSurface2D(
                context.raw,
                TEXTURE_RECTANGLE,
                RGBA,
                width,
                height,
                BGRA,
                UNSIGNED_INT_8_8_8_8_REV,
                (&*surface as *const IOSurfaceRef).cast(),
                0,
            );
            glBindTexture(TEXTURE_RECTANGLE, 0);
            if status != 0 {
                glDeleteTextures(1, &texture);
                return Err(format!("OpenGL does not take the surface (CGL error {status})"));
            }
            glGenFramebuffers(1, &mut framebuffer);
            glBindFramebuffer(FRAMEBUFFER, framebuffer);
            glFramebufferTexture2D(FRAMEBUFFER, COLOR_ATTACHMENT0, TEXTURE_RECTANGLE, texture, 0);
            let complete = glCheckFramebufferStatus(FRAMEBUFFER);
            glBindFramebuffer(FRAMEBUFFER, 0);
            if complete != FRAMEBUFFER_COMPLETE {
                glDeleteFramebuffers(1, &framebuffer);
                glDeleteTextures(1, &texture);
                return Err(format!("the framebuffer around the surface is not complete (0x{complete:x})"));
            }
        }
        Ok(Surface { width, height, texture, framebuffer, shown })
    }

    /// OpenGL's objects go; with the context that made them entered. The surface's memory lives on
    /// for as long as a page shows the texture.
    pub fn release(self) {
        // SAFETY: our own objects, in the context that made them.
        unsafe {
            glDeleteFramebuffers(1, &self.framebuffer);
            glDeleteTextures(1, &self.texture);
        }
    }
}
