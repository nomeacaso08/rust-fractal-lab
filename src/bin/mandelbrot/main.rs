// Initial code based on https://github.com/remexre/mandelbrot-rust-gl

use glium::glutin::dpi::LogicalSize;
use glium::glutin::event::{Event, WindowEvent};
use glium::glutin::event_loop::{ControlFlow, EventLoop};
use glium::glutin::window::WindowBuilder;
use glium::glutin::ContextBuilder;
use glium::index::{NoIndices, PrimitiveType};
use glium::uniforms::{UniformValue, Uniforms};
use glium::{Display, Program, Surface, VertexBuffer};
use rust_fractal_lab::vertex::Vertex;

#[derive(Debug)]
struct DrawParams {
    x_min: f64,
    x_max: f64,
    y_min: f64,
    y_max: f64,

    width: f64,
    height: f64,
}

impl DrawParams {
    fn new(dims: (u32, u32)) -> DrawParams {
        DrawParams {
            x_min: -2.0,
            x_max: 1.0,
            y_min: -1.0,
            y_max: 1.0,
            width: dims.0 as f64,
            height: dims.1 as f64,
        }
    }
}

impl Uniforms for DrawParams {
    fn visit_values<'a, F: FnMut(&str, UniformValue<'a>)>(&'a self, mut f: F) {
        f("xMin", UniformValue::Double(self.x_min));
        f("xMax", UniformValue::Double(self.x_max));
        f("yMin", UniformValue::Double(self.y_min));
        f("yMax", UniformValue::Double(self.y_max));
        f("width", UniformValue::Double(self.width));
        f("height", UniformValue::Double(self.height));
    }
}

fn main() {
    let mut event_loop = EventLoop::new();

    let wb = WindowBuilder::new()
        .with_inner_size(LogicalSize::new(1024.0, 768.0))
        .with_title("Hello world");

    let cb = ContextBuilder::new();

    let display = Display::new(wb, cb, &event_loop).unwrap();

    let vertices = [
        Vertex {
            position: [1.0, -1.0],
        },
        Vertex {
            position: [-1.0, 1.0],
        },
        Vertex {
            position: [-1.0, -1.0],
        },
        Vertex {
            position: [1.0, 1.0],
        },
        Vertex {
            position: [1.0, -1.0],
        },
        Vertex {
            position: [-1.0, 1.0],
        },
    ];

    let vertex_buffer = VertexBuffer::new(&display, &vertices).unwrap();
    let indices = NoIndices(PrimitiveType::TrianglesList);

    let program = Program::from_source(
        &display,
        include_str!("shaders/vertex.glsl"),
        include_str!("shaders/fragment.glsl"),
        None,
    )
    .unwrap();

    let mut draw_params = DrawParams::new(display.get_framebuffer_dimensions());

    event_loop.run(move |ev, _, control_flow| {
        let next_frame_time =
            std::time::Instant::now() + std::time::Duration::from_nanos(16_666_667);

        *control_flow = ControlFlow::WaitUntil(next_frame_time);
        match ev {
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => {
                    *control_flow = ControlFlow::Exit;
                    return;
                }
                _ => return,
            },
            _ => (),
        }

        let mut target = display.draw();
        target.clear_color(0.0, 0.0, 0.0, 1.0);
        target
            .draw(
                &vertex_buffer,
                &indices,
                &program,
                &draw_params,
                &Default::default(),
            )
            .unwrap();
        target.finish().unwrap();
    });
}
