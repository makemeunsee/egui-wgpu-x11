mod x11;

use std::{ffi::c_void, iter, time::Duration};

use anyhow::Result;
use egui::{vec2, Context, Pos2, RawInput};
use egui_demo_lib::DemoWindows;
use egui_wgpu_backend::{RenderPass, ScreenDescriptor};
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
    context: Context,
    raw_input: RawInput,
    demo_app: DemoWindows,
    egui_rpass: RenderPass,
}

impl State {
    fn new(window: &MyWindow) -> Self {
        let size = (window.width, window.height);

        // wgpu stuff

        // The instance is a handle to our GPU
        // BackendBit::PRIMARY => Vulkan + Metal + DX12 + Browser WebGPU
        let instance = wgpu::Instance::new(wgpu::Backends::all());
        let surface = unsafe { instance.create_surface(window) };
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .unwrap();

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                features: wgpu::Features::default(),
                limits: wgpu::Limits::default(),
            },
            None,
        ))
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

        // egui stuff

        let scale_factor = 2.;
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::new(50., 50.),
                vec2(size.0 as f32 - 100., size.1 as f32 - 100.) / scale_factor,
            )),
            pixels_per_point: Some(scale_factor),
            ..Default::default()
        };

        let surface_format = surface.get_supported_formats(&adapter)[0];
        // We use the egui_wgpu_backend crate as the render backend.
        let egui_rpass = RenderPass::new(&device, surface_format, 1);

        // Display the demo application that ships with egui.
        let demo_app = egui_demo_lib::DemoWindows::default();

        let context = Context::default();
        // context.set_fonts(_);
        // context.set_style(_);

        Self {
            surface,
            device,
            queue,
            config,
            size,
            context,
            raw_input,
            demo_app,
            egui_rpass,
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

    fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        let output_frame = self.surface.get_current_texture().unwrap();
        let output_view = output_frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // Begin to draw the UI frame.
        let scale_factor = self.raw_input.pixels_per_point.unwrap_or(1.);
        self.context.begin_frame(self.raw_input.take());
        self.raw_input.pixels_per_point = Some(scale_factor);

        // Draw the demo application.
        self.demo_app.ui(&self.context);

        // End the UI frame. We could now handle the output and draw the UI with the backend.
        let full_output = self.context.end_frame();
        let paint_jobs = self.context.tessellate(full_output.shapes);

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("encoder"),
            });

        // Upload all resources for the GPU.
        let screen_descriptor = ScreenDescriptor {
            physical_width: self.config.width,
            physical_height: self.config.height,
            scale_factor,
        };
        let tdelta: egui::TexturesDelta = full_output.textures_delta;
        self.egui_rpass
            .add_textures(&self.device, &self.queue, &tdelta)
            .expect("add texture ok");
        self.egui_rpass
            .update_buffers(&self.device, &self.queue, &paint_jobs, &screen_descriptor);

        // Record all render passes.
        self.egui_rpass
            .execute(
                &mut encoder,
                &output_view,
                &paint_jobs,
                &screen_descriptor,
                Some(wgpu::Color {
                    r: 0.2,
                    g: 0.1,
                    b: 0.3,
                    a: 0.2,
                }),
            )
            .unwrap();
        // Submit the commands.
        self.queue.submit(iter::once(encoder.finish()));

        // Redraw egui
        output_frame.present();

        self.egui_rpass
            .remove_textures(tdelta)
            .expect("remove texture ok");

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

fn main() -> Result<()> {
    let (conn, screen_num) = x11rb::xcb_ffi::XCBConnection::connect(None)?;

    xfixes_init(&conn);

    let screen = &conn.setup().roots[screen_num];

    let win_id = create_overlay_window(
        &conn,
        screen,
        100,
        100,
        screen.width_in_pixels - 200,
        screen.height_in_pixels - 200,
    )?;

    conn.map_window(win_id)?;
    conn.flush()?;

    let window = MyWindow {
        window: win_id,
        visual_id: screen.root_visual,
        connection: conn.get_raw_xcb_connection(),
        screen: screen_num as i32,
        width: screen.width_in_pixels as u32 - 200,
        height: screen.height_in_pixels as u32 - 200,
    };

    let mut state = State::new(&window);

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
