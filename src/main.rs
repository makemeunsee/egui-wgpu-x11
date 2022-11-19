mod x11;

use std::{ffi::c_void, iter, time::Duration};

use anyhow::Result;
use raw_window_handle::{
    HasRawDisplayHandle, HasRawWindowHandle, RawDisplayHandle, RawWindowHandle, XcbDisplayHandle,
    XcbWindowHandle,
};
use x11::{create_overlay_window, raise_if_not_top, xfixes_init};
use x11rb::{connection::Connection, protocol::xproto::ConnectionExt};

struct MyWindow {
    pub window: u32,
    pub visual_id: u32,
    pub connection: *mut c_void,
    pub screen: i32,
    pub width: u32,
    pub height: u32,
}

struct State {
    surface: wgpu::Surface,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: (u32, u32),
}

impl State {
    async fn new(window: &MyWindow) -> Self {
        let size = (window.width, window.height);

        // The instance is a handle to our GPU
        // BackendBit::PRIMARY => Vulkan + Metal + DX12 + Browser WebGPU
        let instance = wgpu::Instance::new(wgpu::Backends::all());
        let surface = unsafe { instance.create_surface(window) };
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .unwrap();

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: None,
                    features: wgpu::Features::empty(),
                    // WebGL doesn't support all of wgpu's features, so if
                    // we're building for the web we'll have to disable some.
                    limits: if cfg!(target_arch = "wasm32") {
                        wgpu::Limits::downlevel_webgl2_defaults()
                    } else {
                        wgpu::Limits::default()
                    },
                },
                // Some(&std::path::Path::new("trace")), // Trace path
                None,
            )
            .await
            .unwrap();

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface.get_supported_formats(&adapter)[0],
            width: size.0,
            height: size.1,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
        };
        surface.configure(&device, &config);

        Self {
            surface,
            device,
            queue,
            config,
            size,
        }
    }

    pub fn resize(&mut self, new_size: (u32, u32)) {
        if new_size.0 > 0 && new_size.1 > 0 {
            self.size = new_size;
            self.config.width = new_size.0;
            self.config.height = new_size.1;
            self.surface.configure(&self.device, &self.config);
        }
    }

    fn update(&mut self) {}

    fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        {
            let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.1,
                            g: 0.2,
                            b: 0.3,
                            a: 0.5,
                        }),
                        store: true,
                    },
                })],
                depth_stencil_attachment: None,
            });
        }

        self.queue.submit(iter::once(encoder.finish()));
        output.present();

        Ok(())
    }
}

unsafe impl HasRawWindowHandle for MyWindow {
    fn raw_window_handle(&self) -> RawWindowHandle {
        let mut handle = XcbWindowHandle::empty();
        handle.visual_id = self.visual_id;
        handle.window = self.window;
        RawWindowHandle::Xcb(handle)
    }
}
unsafe impl HasRawDisplayHandle for MyWindow {
    fn raw_display_handle(&self) -> RawDisplayHandle {
        let mut handle = XcbDisplayHandle::empty();
        handle.connection = self.connection;
        handle.screen = self.screen;
        RawDisplayHandle::Xcb(handle)
    }
}

// TODO try egui on top: https://github.com/hasenbanck/egui_example/blob/master/src/main.rs

fn main() -> Result<()> {
    let (conn, screen_num) = x11rb::xcb_ffi::XCBConnection::connect(None)?;

    xfixes_init(&conn);

    let screen = &conn.setup().roots[screen_num];

    let win_id = create_overlay_window(&conn, screen, 50, 50, 200, 200)?;

    conn.map_window(win_id)?;
    conn.flush()?;

    let window = MyWindow {
        window: win_id,
        visual_id: screen.root_visual,
        connection: conn.get_raw_xcb_connection(),
        screen: screen_num as i32,
        width: 200,
        height: 200,
    };

    let mut state = pollster::block_on(State::new(&window));

    const STACK_CHECK_DELAY: u32 = 30;
    let mut i = 1;
    loop {
        match state.render() {
            Ok(_) => {}
            // Reconfigure the surface if it's lost or outdated
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                state.resize(state.size)
            }
            // The system is out of memory, we should probably quit
            Err(wgpu::SurfaceError::OutOfMemory) => break,

            Err(wgpu::SurfaceError::Timeout) => println!("Surface timeout"),
        }
        if let Some(event) = conn.poll_for_event().unwrap() {
            println!("Event: {:?}", event);
        } else if i == 0 {
            raise_if_not_top(&conn, screen.root, win_id)?;
        }

        i = (i + 1) % STACK_CHECK_DELAY;
        ::std::thread::sleep(Duration::new(0, 1_000_000_000u32 / 60));
    }

    Ok(())
}
