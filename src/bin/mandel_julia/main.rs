// This program is a combined Mandelbrot and Julia set viewer. It implements histogram coloring,
// runtime selection of Julia set function & color scheme, and a few other features.
//
// If you are looking for something simpler, check out the mandelbrot_simple and julia_simple bins.

// Scaling code based on https://github.com/remexre/mandelbrot-rust-gl

use std::time::Instant;

use clap::ArgGroup;
use clap::Parser;
use glium::framebuffer::{MultiOutputFrameBuffer, ToColorAttachment};
use glium::glutin::dpi::{PhysicalPosition, PhysicalSize};
use glium::glutin::event::{
    ElementState, Event, MouseButton, MouseScrollDelta, TouchPhase, VirtualKeyCode, WindowEvent,
};
use glium::glutin::event_loop::{ControlFlow, EventLoop};
use glium::glutin::window::WindowBuilder;
use glium::glutin::ContextBuilder;
use glium::index::{NoIndices, PrimitiveType};
use glium::program::ShaderStage;
use glium::texture::UnsignedTexture2d;
use glium::uniforms::{UniformValue, Uniforms};
use glium::{Display, Program, Surface, Texture2d, VertexBuffer};
use hdrhistogram::Histogram;
use imgui::{Condition, Context};
use imgui_glium_renderer::Renderer;
use imgui_winit_support::{HiDpiMode, WinitPlatform};
use ouroboros::self_referencing;
use rust_fractal_lab::args::{ColorScheme, JuliaFunction};
use rust_fractal_lab::shader_builder::build_shader;
use rust_fractal_lab::vertex::Vertex;
use strum::VariantNames;

#[derive(Parser)]
#[command(group(
ArgGroup::new("mode")
.args(["is_mandelbrot"])
.conflicts_with("julia_function"),
))]
pub struct MandelJuliaArgs {
    #[arg(short = 'm', long = "mandelbrot", default_value_t = false)]
    is_mandelbrot: bool,

    #[arg(value_enum, default_value_t = JuliaFunction::default())]
    julia_function: JuliaFunction,

    #[arg(value_enum, default_value_t = ColorScheme::Turbo, short, long)]
    color_scheme: ColorScheme,
}

pub struct Dt {
    color_texture: Texture2d,
    iteration_texture: UnsignedTexture2d,
}

#[self_referencing]
struct Data {
    dt: Dt,
    #[borrows(dt)]
    #[covariant]
    buffs: (glium::framebuffer::MultiOutputFrameBuffer<'this>, &'this Dt),
}

#[derive(Debug, Default)]
struct DrawParams {
    x_min: f64,
    x_max: f64,
    y_min: f64,
    y_max: f64,

    width: f32,
    height: f32,
    max_iterations: u32,
    ranges: [u32; 4],
    ranges_2: [u32; 4],
    color_map: String,
    f: String,
    is_mandelbrot: bool,
}

impl DrawParams {
    fn new(dims: (u32, u32), args: &MandelJuliaArgs) -> DrawParams {
        let mut ret = DrawParams {
            width: dims.0 as f32,
            height: dims.1 as f32,
            max_iterations: match args.julia_function {
                JuliaFunction::Snowflakes => 27,
                _ => 1024,
            },
            ranges: [0; 4],
            ranges_2: [0; 4],
            f: args.julia_function.subroutine_name(),
            color_map: args.color_scheme.subroutine_name(),
            is_mandelbrot: args.is_mandelbrot,
            ..DrawParams::default()
        };

        ret.reset(args.is_mandelbrot);
        ret
    }

    fn reset(&mut self, is_mandelbrot: bool) {
        self.x_min = -2.0;
        self.x_max = {
            if is_mandelbrot {
                1.0
            } else {
                2.0
            }
        };
        self.y_min = {
            if is_mandelbrot {
                -1.0
            } else {
                -2.0
            }
        };
        self.y_max = {
            if is_mandelbrot {
                1.0
            } else {
                2.0
            }
        };
    }

    fn scroll(&mut self, x: f64, y: f64) {
        let s_x = (self.x_max - self.x_min) / 10.0;
        let s_y = (self.y_max - self.y_min) / 10.0;
        self.x_min += x * s_x;
        self.x_max += x * s_x;
        self.y_min += y * s_y;
        self.y_max += y * s_y;
    }

    fn pan(&mut self, x: f64, y: f64) {
        self.scroll(x / 100.0, y / 100.0)
    }

    fn zoom_in(&mut self) {
        let s_x = (self.x_max - self.x_min) / 10.0;
        let s_y = (self.y_max - self.y_min) / 10.0;
        self.x_min += s_x;
        self.x_max -= s_x;
        self.y_min += s_y;
        self.y_max -= s_y;
    }

    fn zoom_out(&mut self) {
        let s_x = (self.x_max - self.x_min) / 10.0;
        let s_y = (self.y_max - self.y_min) / 10.0;
        self.x_min -= s_x;
        self.x_max += s_x;
        self.y_min -= s_y;
        self.y_max += s_y;
    }
}

impl Uniforms for DrawParams {
    fn visit_values<'a, F: FnMut(&str, UniformValue<'a>)>(&'a self, mut f: F) {
        f("xMin", UniformValue::Double(self.x_min));
        f("xMax", UniformValue::Double(self.x_max));
        f("yMin", UniformValue::Double(self.y_min));
        f("yMax", UniformValue::Double(self.y_max));
        f("width", UniformValue::Float(self.width));
        f("height", UniformValue::Float(self.height));
        f(
            "max_iterations",
            UniformValue::UnsignedInt(self.max_iterations),
        );
        f("ranges", UniformValue::UnsignedIntVec4(self.ranges));
        f("ranges_2", UniformValue::UnsignedIntVec4(self.ranges_2));
        f(
            "ColorMap",
            UniformValue::Subroutine(ShaderStage::Fragment, self.color_map.as_str()),
        );
        f(
            "F",
            UniformValue::Subroutine(ShaderStage::Fragment, self.f.as_str()),
        );
        f(
            "Colorize",
            UniformValue::Subroutine(ShaderStage::Fragment, {
                match self.f.as_str() {
                    "FCloud" => "ColorizeCloud",
                    "FSnowflakes" => "ColorizeSnowflakes",
                    _ => "ColorizeDefault",
                }
            }),
        );
        f("is_mandelbrot", UniformValue::Bool(self.is_mandelbrot));
    }
}

const WINDOW_WIDTH: u32 = 1024;
const WINDOW_HEIGHT: u32 = 768;

fn main() {
    let args = MandelJuliaArgs::parse();

    let event_loop = EventLoop::new();

    let wb = WindowBuilder::new()
        .with_inner_size(PhysicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT))
        .with_resizable(false)
        .with_title("Mandelbrot / Julia set viewer")
        .with_position(PhysicalPosition::new(0, 0));

    let cb = ContextBuilder::new();
    let main_display = Display::new(wb, cb, &event_loop).unwrap();

    // This had to be a separate window (unlike in the bifurcation bin), otherwise blitting
    // didn't work for me.
    let wb = WindowBuilder::new()
        .with_title("Parameters")
        .with_resizable(false)
        .with_position(PhysicalPosition::new(
            main_display.gl_window().window().inner_size().width,
            0,
        ));
    let cb = ContextBuilder::new();
    let params_display = Display::new(wb, cb, &event_loop).unwrap();

    let mut imgui = Context::create();
    imgui.set_ini_filename(None);

    let mut platform = WinitPlatform::init(&mut imgui);
    let gl_params_window = params_display.gl_window();
    let params_window = gl_params_window.window();
    platform.attach_window(imgui.io_mut(), params_window, HiDpiMode::Default);
    drop(gl_params_window);

    let vertices: [Vertex; 6] = [
        [1.0, -1.0].into(),
        [-1.0, 1.0].into(),
        [-1.0, -1.0].into(),
        [1.0, 1.0].into(),
        [1.0, -1.0].into(),
        [-1.0, 1.0].into(),
    ];

    let vertex_buffer = VertexBuffer::new(&main_display, &vertices).unwrap();
    let indices = NoIndices(PrimitiveType::TrianglesList);

    let program = Program::from_source(
        &main_display,
        r##"#version 140
in vec2 position;
void main() {
	gl_Position = vec4(position, 0.0, 1.0);
}
"##,
        &build_shader(include_str!("shaders/fragment.glsl")),
        None,
    )
    .unwrap();

    let iteration_texture = UnsignedTexture2d::empty_with_format(
        &main_display,
        glium::texture::UncompressedUintFormat::U32U32,
        glium::texture::MipmapsOption::NoMipmap,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
    )
    .unwrap();

    iteration_texture
        .as_surface()
        .clear_color(0.0, 0.0, 0.0, 0.0);

    let color_texture = Texture2d::empty_with_format(
        &main_display,
        glium::texture::UncompressedFloatFormat::F16F16F16F16,
        glium::texture::MipmapsOption::NoMipmap,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
    )
    .unwrap();

    let mut tenants = DataBuilder {
        dt: Dt {
            color_texture,
            iteration_texture,
        },
        buffs_builder: |dt| {
            let output = [
                ("color", dt.color_texture.to_color_attachment()),
                (
                    "pixel_iterations",
                    dt.iteration_texture.to_color_attachment(),
                ),
            ];
            let framebuffer = MultiOutputFrameBuffer::new(&main_display, output).unwrap();
            (framebuffer, dt)
        },
    }
    .build();

    let dim = main_display.get_framebuffer_dimensions();
    eprintln!("{:?}", dim);
    let mut draw_params = DrawParams::new(main_display.get_framebuffer_dimensions(), &args);

    // Input variables
    let mut mouse_down = false;
    let mut mouse_last = (0f64, 0f64);

    let mut renderer =
        Renderer::init(&mut imgui, &params_display).expect("Failed to initialize renderer");
    let mut last_frame = Instant::now();

    // Create histogram using 3 significant figures (crate's recommended default)
    let mut hist = Histogram::<u32>::new(3).unwrap();

    let mut selected_julia_func = JuliaFunction::VARIANTS
        .iter()
        .position(|i| i == &args.julia_function.to_string())
        .unwrap_or_default();
    let mut selected_color_map = ColorScheme::VARIANTS
        .iter()
        .position(|i| i == &args.color_scheme.to_string())
        .unwrap_or_default();

    event_loop.run(move |ev, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match &ev {
            Event::NewEvents(_) => {
                let now = Instant::now();
                imgui.io_mut().update_delta_time(now - last_frame);
                last_frame = now;
            }
            Event::MainEventsCleared => {
                let gl_params_window = params_display.gl_window();
                platform
                    .prepare_frame(imgui.io_mut(), gl_params_window.window())
                    .expect("Failed to prepare frame");
                gl_params_window.window().request_redraw();
            }
            Event::RedrawRequested(window_id) => {
                if *window_id == main_display.gl_window().window().id() {
                    tenants.with_mut(|fields| {
                        let framebuffer = &mut fields.buffs.0;
                        let dt = fields.dt;

                        framebuffer
                            .draw(
                                &vertex_buffer,
                                indices,
                                &program,
                                &draw_params,
                                &Default::default(),
                            )
                            .unwrap();

                        main_display.assert_no_error(None);

                        // This call to unchecked_read requires our fork of glium. If you try vanilla
                        // glium, it will fail to compile.
                        let p: Vec<Vec<(u32, u32)>> =
                            unsafe { dt.iteration_texture.unchecked_read() };

                        // Populate histogram
                        hist.reset();
                        for p in p.into_iter().flatten().filter(|b| b.1 != 1) {
                            hist.record(p.0 as u64).unwrap();
                        }

                        // Compute the octiles (8-quantiles)
                        let mut octiles = (0..=8)
                            .map(|i| hist.value_at_quantile(i as f64 / 8.0))
                            .collect::<Vec<_>>();

                        // Try to nudge identical values to the next value
                        let max = hist.max();
                        for i in 0..7 {
                            octiles[i + 1] = octiles[i].max(octiles[i + 1]);
                            if octiles[i] == octiles[i + 1] {
                                octiles[i + 1] = hist.next_non_equivalent(octiles[i + 1]).min(max);
                            }
                        }

                        let octiles = octiles.into_iter().map(|v| v as u32).collect::<Vec<_>>();

                        draw_params.ranges = octiles[0..4].try_into().unwrap();
                        draw_params.ranges_2 = octiles[4..8].try_into().unwrap();

                        eprintln!("{:?} {:?}", draw_params.ranges, draw_params.ranges_2);

                        let mut target = main_display.draw();
                        target.clear_color_srgb(1.0, 1.0, 1.0, 1.0);

                        if cfg!(windows) {
                            // Re-draw fractal using updated iteration counts
                            framebuffer
                                .draw(
                                    &vertex_buffer,
                                    indices,
                                    &program,
                                    &draw_params,
                                    &Default::default(),
                                )
                                .unwrap();

                            // Blit the pixels to the surface
                            dt.color_texture
                                .as_surface()
                                .fill(&target, glium::uniforms::MagnifySamplerFilter::Linear);
                        } else {
                            // TODO: at least on Ubuntu on VMware, blitting doesn't work here.
                            // Workaround for Linux: re-execute the shader, this time targeting the surface
                            target
                                .draw(
                                    &vertex_buffer,
                                    indices,
                                    &program,
                                    &draw_params,
                                    &Default::default(),
                                )
                                .unwrap();
                        }

                        target.finish().expect("Failed to swap buffers");
                    });
                } else {
                    let mut params_target = params_display.draw();
                    params_target.clear_color_srgb(1.0, 1.0, 1.0, 1.0);

                    let ui = imgui.frame();

                    ui.window("Controls")
                        .always_auto_resize(true)
                        .position([0.0, 0.0], Condition::FirstUseEver)
                        .build(|| {
                            let mut changed = false;

                            // TODO: Only recalculate when the histogram actually changes
                            // TODO: allocate vec once, then reuse
                            let mut p = Vec::with_capacity(hist.max() as usize + 1);
                            for i in 0..=hist.max() {
                                p.push(hist.count_at(i) as f32);
                            }

                            ui.plot_histogram("Escape iteration counts", p.as_slice())
                                .graph_size([300.0, 100.0])
                                .build();

                            changed |= {
                                let mandelbrot_changed =
                                    ui.checkbox("Mandelbrot mode", &mut draw_params.is_mandelbrot);
                                if mandelbrot_changed {
                                    draw_params.reset(draw_params.is_mandelbrot);
                                }
                                mandelbrot_changed
                            };

                            ui.disabled(draw_params.is_mandelbrot, || {
                                let func_changed = ui.combo_simple_string(
                                    "Julia function",
                                    &mut selected_julia_func,
                                    JuliaFunction::VARIANTS,
                                );
                                if func_changed {
                                    draw_params.f = format!(
                                        "F{}",
                                        JuliaFunction::VARIANTS[selected_julia_func]
                                    );
                                    if draw_params.f == "FSnowflakes" {
                                        draw_params.max_iterations = 27;
                                    } else {
                                        draw_params.max_iterations = 1024;
                                    }
                                }
                                changed |= func_changed;
                            });

                            changed |= {
                                let map_changed = ui.combo_simple_string(
                                    "Color map",
                                    &mut selected_color_map,
                                    ColorScheme::VARIANTS,
                                );
                                if map_changed {
                                    draw_params.color_map = format!(
                                        "ColorMap{}",
                                        ColorScheme::VARIANTS[selected_color_map]
                                    );
                                }
                                map_changed
                            };

                            changed |= ui.input_scalar("x_max", &mut draw_params.x_max).build();
                            changed |=
                                ui.slider("iterations", 1, 1024, &mut draw_params.max_iterations);

                            if changed {
                                main_display.gl_window().window().request_redraw();
                            }
                        });

                    let gl_params_window = params_display.gl_window();
                    // TODO doesn't seem to work
                    // gl_params_window
                    //     .window()
                    //     .set_inner_size(LogicalSize::<u32>::from(ui.window_size()));

                    platform.prepare_render(ui, gl_params_window.window());
                    let draw_data = imgui.render();

                    renderer
                        .render(&mut params_target, draw_data)
                        .expect("Rendering failed");

                    params_target.finish().expect("Failed to swap buffers");
                }
            }
            outer @ Event::WindowEvent { window_id, .. }
                if *window_id == params_display.gl_window().window().id() =>
            {
                let gl_window = params_display.gl_window();
                platform.handle_event(imgui.io_mut(), gl_window.window(), outer);
            }
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::MouseInput {
                    state,
                    button: MouseButton::Left,
                    ..
                } => {
                    mouse_down = match state {
                        ElementState::Pressed => true,
                        ElementState::Released => false,
                    }
                }
                WindowEvent::CursorMoved { position, .. } => {
                    if mouse_down {
                        main_display.gl_window().window().request_redraw();
                        draw_params.pan(mouse_last.0 - position.x, position.y - mouse_last.1);
                    }

                    mouse_last = (position.x, position.y);

                    if !mouse_down {}
                }
                WindowEvent::MouseWheel {
                    phase: TouchPhase::Moved,
                    delta: MouseScrollDelta::LineDelta(_x, y),
                    ..
                } => {
                    main_display.gl_window().window().request_redraw();
                    if *y < 0.0 {
                        draw_params.zoom_out()
                    } else {
                        draw_params.zoom_in()
                    }
                }
                WindowEvent::KeyboardInput { input, .. }
                    if input.state == ElementState::Pressed =>
                {
                    if let Some(keycode) = input.virtual_keycode {
                        match keycode {
                            VirtualKeyCode::Minus => draw_params.zoom_out(),
                            VirtualKeyCode::Equals => draw_params.zoom_in(),
                            VirtualKeyCode::Space => draw_params.reset(draw_params.is_mandelbrot),
                            VirtualKeyCode::Up => draw_params.scroll(0.0, -1.0),
                            VirtualKeyCode::Left => draw_params.scroll(-1.0, 0.0),
                            VirtualKeyCode::Right => draw_params.scroll(1.0, 0.0),
                            VirtualKeyCode::Down => draw_params.scroll(0.0, 1.0),
                            _ => return,
                        }

                        main_display.gl_window().window().request_redraw();
                    }
                }
                WindowEvent::CloseRequested => {
                    *control_flow = ControlFlow::Exit;
                }
                _ => {}
            },
            _ => (),
        }
    });
}
