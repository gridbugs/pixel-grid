#[macro_use]
extern crate gfx;
extern crate coord_2d;
extern crate gfx_device_gl;
extern crate gfx_window_glutin;
extern crate glutin;
extern crate grid_2d;

pub use coord_2d::{Coord, Size};
use gfx::traits::FactoryExt;
use gfx::Device;
use gfx::Factory;
use grid_2d::coord_system::{CoordSystem, XThenY, XThenYIter};
use std::iter;
use std::slice;

type ColourFormat = gfx::format::Srgba8;
type DepthFormat = gfx::format::DepthStencil;

const QUAD_INDICES: [u16; 6] = [0, 1, 2, 2, 3, 0];
const QUAD_COORDS: [[f32; 2]; 4] = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];

gfx_vertex_struct!(QuadCorners {
    corner_zero_to_one: [f32; 2] = "a_CornerZeroToOne",
});

gfx_vertex_struct!(Cell {
    coord: [f32; 2] = "a_Coord",
    colour: [f32; 3] = "a_Colour",
});

gfx_constant_struct!(Properties {
    window_size_in_pixels: [f32; 2] = "u_WindowSizeInPixels",
    cell_size_in_pixels: [f32; 2] = "u_CellSizeInPixels",
});

gfx_pipeline!(pipe {
    quad_corners: gfx::VertexBuffer<QuadCorners> = (),
    cells: gfx::InstanceBuffer<Cell> = (),
    properties: gfx::ConstantBuffer<Properties> = "Properties",
    out_colour: gfx::BlendTarget<ColourFormat> =
        ("Target", gfx::state::ColorMask::all(), gfx::preset::blend::ALPHA),
});

pub struct Window {
    encoder: gfx::Encoder<gfx_device_gl::Resources, gfx_device_gl::CommandBuffer>,
    device: gfx_device_gl::Device,
    factory: gfx_device_gl::Factory,
    window: glutin::GlWindow,
    events_loop: glutin::EventsLoop,
    bundle:
        gfx::pso::bundle::Bundle<gfx_device_gl::Resources, pipe::Data<gfx_device_gl::Resources>>,
    cell_upload: gfx::handle::Buffer<gfx_device_gl::Resources, Cell>,
    num_cells: usize,
    coord_system: XThenY,
    closed: bool,
}

pub struct PixelGrid<'a> {
    coord_system: XThenY,
    writer: gfx::mapping::Writer<'a, gfx_device_gl::Resources, Cell>,
}

pub struct Pixel<'a> {
    cell: &'a mut Cell,
}

impl<'a> Pixel<'a> {
    pub fn set_colour_array_f32(&mut self, colour: [f32; 3]) {
        self.cell.colour = colour;
    }
    pub fn set_colour_array_u8(&mut self, [r, g, b]: [u8; 3]) {
        self.set_colour_array_f32([r as f32 / 255., g as f32 / 255., b as f32 / 255.]);
    }
}

pub struct WindowSpec {
    pub title: String,
    pub grid_size: Size,
    pub cell_size: Size,
}

impl WindowSpec {
    pub fn new(title: String, grid_size: Size, cell_size: Size) -> Self {
        Self {
            title,
            grid_size,
            cell_size,
        }
    }
}

impl Window {
    pub fn new(
        WindowSpec {
            title,
            grid_size,
            cell_size,
        }: WindowSpec,
    ) -> Self {
        let events_loop = glutin::EventsLoop::new();
        let size_in_pixels =
            Size::new(grid_size.x() * cell_size.x(), grid_size.y() * cell_size.y());
        let glutin_size =
            glutin::dpi::LogicalSize::new(size_in_pixels.x() as f64, size_in_pixels.y() as f64);
        let builder = glutin::WindowBuilder::new()
            .with_title(title)
            .with_resizable(false)
            .with_dimensions(glutin_size)
            .with_max_dimensions(glutin_size)
            .with_min_dimensions(glutin_size);
        let context = glutin::ContextBuilder::new().with_vsync(true);
        let (window, device, mut factory, rtv, _dsv) =
            gfx_window_glutin::init::<ColourFormat, DepthFormat>(builder, context, &events_loop)
                .expect("Failed to create window");
        let (width, height): (u32, u32) = window.get_outer_size().unwrap().into();
        let mut encoder: gfx::Encoder<gfx_device_gl::Resources, gfx_device_gl::CommandBuffer> =
            factory.create_command_buffer().into();
        let pso = factory
            .create_pipeline_simple(
                r#"
                #version 150 core
                uniform Properties {
                    vec2 u_WindowSizeInPixels;
                    vec2 u_CellSizeInPixels;
                };
                in vec2 a_CornerZeroToOne;
                in vec2 a_Coord;
                in vec3 a_Colour;
                flat out vec3 v_Colour;
                void main() {
                    v_Colour = a_Colour;
                    vec2 top_left_corner_in_pixels = (a_Coord + a_CornerZeroToOne) * u_CellSizeInPixels;
                    float x = ((top_left_corner_in_pixels.x / u_WindowSizeInPixels.x) * 2) - 1;
                    float y = 1 - ((top_left_corner_in_pixels.y / u_WindowSizeInPixels.y) * 2);
                    gl_Position = vec4(x, y, 0, 1);
                }
                "#.as_bytes(),
                r#"
                #version 150 core
                out vec4 Target;
                flat in vec3 v_Colour;
                void main() {
                    Target = vec4(v_Colour, 1);
                }
                "#.as_bytes(),
                pipe::new(),
            )
            .expect("Failed to create pipeline");
        let quad_corners_data = QUAD_COORDS
            .iter()
            .map(|v| QuadCorners {
                corner_zero_to_one: *v,
            })
            .collect::<Vec<_>>();
        let (quad_corners_buffer, mut slice) =
            factory.create_vertex_buffer_with_slice(&quad_corners_data, &QUAD_INDICES[..]);
        let num_cells = grid_size.count();
        slice.instances = Some((num_cells as u32, 0));
        let cell_buffer: gfx::handle::Buffer<gfx_device_gl::Resources, Cell> = factory
            .create_buffer(
                num_cells,
                gfx::buffer::Role::Vertex,
                gfx::memory::Usage::Data,
                gfx::memory::Bind::TRANSFER_DST,
            )
            .expect("Failed to create instance buffer");
        let cell_upload: gfx::handle::Buffer<gfx_device_gl::Resources, Cell> = factory
            .create_upload_buffer(num_cells)
            .expect("Failed to create instance upload buffer");
        for (coord, cell) in XThenYIter::from(grid_size).zip(
            factory
                .write_mapping(&cell_upload)
                .expect("Failed to map instance upload buffer")
                .iter_mut(),
        ) {
            cell.coord = [coord.x as f32, coord.y as f32];
            cell.colour = [0., 0., 1.];
        }
        let properties_buffer: gfx::handle::Buffer<gfx_device_gl::Resources, Properties> =
            factory.create_constant_buffer(1);
        let data = pipe::Data {
            quad_corners: quad_corners_buffer,
            cells: cell_buffer,
            properties: properties_buffer,
            out_colour: rtv,
        };
        encoder.update_constant_buffer(
            &data.properties,
            &Properties {
                window_size_in_pixels: [width as f32, height as f32],
                cell_size_in_pixels: [cell_size.x() as f32, cell_size.y() as f32],
            },
        );
        let bundle = gfx::pso::bundle::Bundle::new(slice, pso, data);
        Self {
            encoder,
            device,
            factory,
            window,
            events_loop,
            bundle,
            cell_upload,
            num_cells,
            coord_system: XThenY::from(grid_size),
            closed: false,
        }
    }
    pub fn draw(&mut self) {
        let mut closed = false;
        self.events_loop.poll_events(|event| match event {
            glutin::Event::WindowEvent { event, .. } => match event {
                glutin::WindowEvent::CloseRequested => {
                    closed = true;
                }
                _ => (),
            },
            _ => (),
        });
        self.closed = self.closed || closed;
        if self.closed {
            return;
        }
        self.encoder
            .clear(&self.bundle.data.out_colour, [0., 1., 0., 0.]);
        self.encoder
            .copy_buffer(
                &self.cell_upload,
                &self.bundle.data.cells,
                0,
                0,
                self.num_cells,
            )
            .expect("Failed to copy cells");
        self.bundle.encode(&mut self.encoder);
        self.encoder.flush(&mut self.device);
        self.window.swap_buffers().unwrap();
        self.device.cleanup();
    }
    pub fn is_closed(&self) -> bool {
        self.closed
    }
    pub fn pixel_grid(&mut self) -> PixelGrid {
        let writer = self.factory
            .write_mapping(&self.cell_upload)
            .expect("Failed to map instance upload buffer");
        PixelGrid {
            writer,
            coord_system: self.coord_system.clone(),
        }
    }
    pub fn with_pixel_grid<F: FnMut(PixelGrid)>(&mut self, mut f: F) {
        f(self.pixel_grid())
    }
}

pub struct IterMut<'a> {
    iter: slice::IterMut<'a, Cell>,
}

impl<'a> Iterator for IterMut<'a> {
    type Item = Pixel<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next().map(|cell| Pixel { cell })
    }
}

pub type EnumerateMut<'a> = iter::Zip<XThenYIter, IterMut<'a>>;

pub type CoordIter = XThenYIter;

impl<'a> PixelGrid<'a> {
    pub fn size(&self) -> Size {
        self.coord_system.size()
    }
    pub fn width(&self) -> u32 {
        self.size().width()
    }
    pub fn height(&self) -> u32 {
        self.size().height()
    }
    pub fn len(&self) -> usize {
        self.size().count()
    }
    pub fn iter_mut(&mut self) -> IterMut {
        IterMut {
            iter: self.writer.iter_mut(),
        }
    }
    pub fn coord_iter(&self) -> CoordIter {
        self.coord_system.coord_iter()
    }
    pub fn enumerate_mut(&mut self) -> EnumerateMut {
        self.coord_iter().zip(self.iter_mut())
    }
    pub fn get_mut(&mut self, coord: Coord) -> Option<Pixel> {
        self.coord_system
            .index_of_coord(coord)
            .map(move |index| Pixel {
                cell: &mut self.writer[index],
            })
    }
    pub fn get_checked_mut(&mut self, coord: Coord) -> Pixel {
        let index = self.coord_system.index_of_coord_checked(coord);
        Pixel {
            cell: &mut self.writer[index],
        }
    }
    pub fn index_of_coord(&self, coord: Coord) -> Option<usize> {
        self.coord_system.index_of_coord(coord)
    }
    pub fn get_index_mut(&mut self, index: usize) -> Pixel {
        Pixel {
            cell: &mut self.writer[index],
        }
    }
}
