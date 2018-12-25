extern crate pixel_grid;

use pixel_grid::{Size, Window, WindowSpec};

fn main() {
    let mut pg = Window::new(WindowSpec {
        title: "gradient".to_string(),
        grid_size: Size::new(20, 20),
        cell_size: Size::new(8, 16),
    });
    loop {
        if pg.is_window_closed() {
            break;
        }
        pg.with_pixel_grid(|mut pixel_grid| {
            let width = pixel_grid.width() as f32;
            let height = pixel_grid.height() as f32;
            pixel_grid.enumerate_mut().for_each(|(coord, mut pixel)| {
                let x = coord.x as f32 / width;
                let y = coord.y as f32 / height;
                pixel.set_colour_rgb(x, y, 1.);
            });
        });
        pg.draw();
        ::std::thread::sleep(::std::time::Duration::from_millis(16));
    }
}
