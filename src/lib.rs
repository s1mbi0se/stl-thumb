extern crate cgmath;
#[macro_use]
extern crate glium;
extern crate image;
#[macro_use]
extern crate log;
extern crate mint;

pub mod config;
mod mesh;
mod fxaa;

use std::error::Error;
use std::fs::File;
use std::{io, thread, time};
use config::Config;
use cgmath::EuclideanSpace;
use glium::{glutin, Surface, CapabilitiesSource};
use mesh::Mesh;

// TODO: Move this stuff to config module
const BACKGROUND_COLOR: (f32, f32, f32, f32) = (1.0, 1.0, 1.0, 0.0);
const CAM_FOV_DEG: f32 = 30.0;
const CAM_POSITION: cgmath::Point3<f32> = cgmath::Point3 {x: 2.0, y: -4.0, z: 2.0};


struct Material {
    ambient: [f32; 3],
    diffuse: [f32; 3],
    specular: [f32; 3],
}


fn print_matrix(m: [[f32; 4]; 4]) {
    for i in 0..4 {
        debug!("{:.3}\t{:.3}\t{:.3}\t{:.3}", m[i][0], m[i][1], m[i][2], m[i][3]);
    }
    debug!("");
}


pub fn run(config: &Config) -> Result<(), Box<Error>> {
    // Create geometry from STL file
    // =========================

    // TODO: Add support for URIs instead of plain file names
    // https://developer.gnome.org/integration-guide/stable/thumbnailer.html.en
    let stl_file = File::open(&config.stl_filename)?;
    let mesh = Mesh::from_stl(stl_file)?;


    // Graphics Stuff
    // ==============

    // Create GL context
    // -----------------

    let mut events_loop = glutin::EventsLoop::new();
    let window_dim = glutin::dpi::LogicalSize::new(
        config.width.into(),
        config.height.into());
    let window = glutin::WindowBuilder::new()
        .with_title("stl-thumb")
        .with_dimensions(window_dim)
        .with_min_dimensions(window_dim)
        .with_max_dimensions(window_dim)
        .with_visibility(config.visible);
    let context = glutin::ContextBuilder::new()
        .with_depth_buffer(24);
        //.with_multisampling(8);
        //.with_gl(glutin::GlRequest::Specific(glutin::Api::OpenGlEs, (2, 0)));
    let display = glium::Display::new(window, context, &events_loop).unwrap();
    //let context = glutin::HeadlessRendererBuilder::new(config.width, config.height)
    //    //.with_depth_buffer(24)
    //    .build().unwrap();
    //let display = glium::HeadlessRenderer::new(context).unwrap();

    // Print context information
    info!("GL Version:   {:?}", display.get_opengl_version());
    info!("GL Version:   {}", display.get_opengl_version_string());
    info!("GLSL Version: {:?}", display.get_supported_glsl_version());
    info!("Vendor:       {}", display.get_opengl_vendor_string());
    info!("Renderer      {}", display.get_opengl_renderer_string());
    info!("Free GPU Mem: {:?}", display.get_free_video_memory());
    info!("Depth Bits:   {:?}\n", display.get_capabilities().depth_bits);


    let params = glium::DrawParameters {
        depth: glium::Depth {
            test: glium::draw_parameters::DepthTest::IfLess,
            write: true,
            .. Default::default()
        },
        backface_culling: glium::draw_parameters::BackfaceCullingMode::CullClockwise,
        .. Default::default()
    };

    // Load and compile shaders
    // ------------------------

    let vertex_shader_src = include_str!("shaders/model.vert");
    let pixel_shader_src = include_str!("shaders/model.frag");

    // TODO: Cache program binary
    let program = glium::Program::from_source(&display, &vertex_shader_src, &pixel_shader_src, None);
    let program = match program {
        Ok(p) => p,
        Err(glium::CompilationError(err)) => {
            error!("{}",err);
            panic!("Compiling shaders");
        },
        Err(err) => panic!("{}",err),
    };

    // Send mesh data to GPU
    // ---------------------

    let vertex_buf = glium::VertexBuffer::new(&display, &mesh.vertices).unwrap();
    let normal_buf = glium::VertexBuffer::new(&display, &mesh.normals).unwrap();
    // Can use NoIndices here because STLs are dumb
    let indices = glium::index::NoIndices(glium::index::PrimitiveType::TrianglesList);

    // Setup uniforms
    // --------------

    // Transformation matrix (positions, scales and rotates model)
    let transform_matrix = mesh.scale_and_center();

    // View matrix (convert to positions relative to camera)
    // TODO: View matrix never changes. We could bake this at compile time and save a
    // little processing.
    let view_matrix = cgmath::Matrix4::look_at(CAM_POSITION, cgmath::Point3::origin(), cgmath::Vector3::unit_z());
    debug!("View:");
    print_matrix(view_matrix.into());

    // Perspective matrix (give illusion of depth)
    let perspective_matrix = cgmath::perspective(
        cgmath::Deg(CAM_FOV_DEG),
        config.width as f32 / config.height as f32,
        0.1,
        1024.0,
    );
    debug!("Perspective:");
    print_matrix(perspective_matrix.into());

    // Direction of light source
    //let light_dir = [-1.4, 0.4, -0.7f32];
    let light_dir = [-1.1, 0.4, 1.0f32];

    // Colors of object
    let colors = Material {
        ambient: [0.0, 0.0, 0.4],
        diffuse: [0.0, 0.5, 1.0],
        specular: [1.0, 1.0, 1.0],
    };

    let uniforms = uniform! {
        model: Into::<[[f32; 4]; 4]>::into(transform_matrix),
        view: Into::<[[f32; 4]; 4]>::into(view_matrix),
        perspective: Into::<[[f32; 4]; 4]>::into(perspective_matrix),
        u_light: light_dir,
        ambient_color: colors.ambient,
        diffuse_color: colors.diffuse,
        specular_color: colors.specular,
    };

    // Draw
    // ----

    // Create off screen texture to render to
    let texture = glium::Texture2d::empty(&display, config.width, config.height).unwrap();
    let depthtexture = glium::texture::DepthTexture2d::empty(&display, config.width, config.height).unwrap();
    let mut framebuffer = glium::framebuffer::SimpleFrameBuffer::with_depth_buffer(&display, &texture, &depthtexture).unwrap();

    // Create FXAA system
    let fxaa = fxaa::FxaaSystem::new(&display);

    fxaa::draw(&fxaa, &mut framebuffer, true, |target| {
        // Fills background color and clears depth buffer
        target.clear_color_and_depth(BACKGROUND_COLOR, 1.0);
        target.draw((&vertex_buf, &normal_buf), &indices, &program, &uniforms, &params)
            .unwrap();
        // TODO: Shadows
    });

    // Save Image
    // ==========

    let pixels: glium::texture::RawImage2d<u8> = texture.read();
    let img = image::ImageBuffer::from_raw(config.width, config.height, pixels.data.into_owned()).unwrap();
    let img = image::DynamicImage::ImageRgba8(img).flipv();
    // Write to stdout if user did not specify a file
    let mut output: Box<io::Write> = match config.img_filename {
        Some(ref x) => {
            Box::new(std::fs::File::create(&x).unwrap())
        },
        None => Box::new(io::stdout()),
    };
    img.write_to(&mut output, image::ImageFormat::PNG)
        .expect("Error saving image");

    // Wait until window is closed
    // ===========================

    if config.visible {
        let mut closed = false;
        let sleep_time = time::Duration::from_millis(10);
        while !closed {
            thread::sleep(sleep_time);
            // Copy framebuffer to display
            // TODO: I think theres some screwy srgb stuff going on here
            let target = display.draw();
            target.blit_from_simple_framebuffer(&framebuffer,
                                                &glium::Rect {
                                                    left: 0,
                                                    bottom: 0,
                                                    width: config.width,
                                                    height: config.height,
                                                },
                                                &glium::BlitTarget {
                                                    left: 0,
                                                    bottom: 0,
                                                    width: config.width as i32,
                                                    height: config.height as i32,
                                                },
                                                glium::uniforms::MagnifySamplerFilter::Nearest);
            target.finish().unwrap();
            // Listing the events produced by the application and waiting to be received
            events_loop.poll_events(|ev| {
                match ev {
                    glutin::Event::WindowEvent { event, .. } => match event {
                        glutin::WindowEvent::CloseRequested => closed = true,
                        glutin::WindowEvent::Destroyed => closed = true,
                        _ => (),
                    },
                    _ => (),
                }
            });
        }
    }

    Ok(())
}


// TODO: Move tests to their own file
#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::ErrorKind;
    use super::*;

    #[test]
    fn cube() {
        let config = Config {
            stl_filename: "test_data/cube.stl".to_string(),
            img_filename: "cube.png".to_string(),
            width: 1024,
            height: 768,
            visible: false,
        };

        match fs::remove_file(&config.img_filename) {
            Ok(_) => (),
            Err(ref error) if error.kind() == ErrorKind::NotFound => (),
            Err(_) => {
                panic!("Couldn't clean files before testing");
            }
        }

        run(&config).expect("Error in run function");

        let size = fs::metadata(config.img_filename)
            .expect("No file created")
            .len();

        assert_ne!(0, size);
    }
}
