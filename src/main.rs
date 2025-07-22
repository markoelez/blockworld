use winit::{
    event::{Event, WindowEvent, ElementState, VirtualKeyCode, DeviceEvent},
    event_loop::{ControlFlow, EventLoop},
    window::{WindowBuilder, CursorGrabMode},
};

mod world;
mod camera;
mod renderer;
mod ui;

use world::World;
use camera::Camera;
use renderer::Renderer;
use ui::Inventory;

fn main() {
    env_logger::init();
    
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("BlockWorld")
        .build(&event_loop)
        .unwrap();
    
    let mut renderer = pollster::block_on(Renderer::new(&window));
    let mut world = World::new();
    let mut camera = Camera::new(&renderer.config);
    let mut inventory = Inventory::new();
    
    let mut last_frame = std::time::Instant::now();
    let mut mouse_captured = true;
    let mut targeted_block: Option<(i32, i32, i32)> = None;
    
    // Start with mouse captured
    if let Err(_) = window.set_cursor_grab(CursorGrabMode::Locked) {
        window.set_cursor_grab(CursorGrabMode::Confined).ok();
    }
    window.set_cursor_visible(false);
    
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Poll;
        
        match event {
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => *control_flow = ControlFlow::Exit,
                WindowEvent::KeyboardInput { input, .. } => {
                    if let Some(keycode) = input.virtual_keycode {
                        let is_pressed = input.state == ElementState::Pressed;
                        
                        if is_pressed && keycode == VirtualKeyCode::Escape {
                            mouse_captured = !mouse_captured;
                            if mouse_captured {
                                if let Err(_) = window.set_cursor_grab(CursorGrabMode::Locked) {
                                    window.set_cursor_grab(CursorGrabMode::Confined).ok();
                                }
                                window.set_cursor_visible(false);
                            } else {
                                window.set_cursor_grab(CursorGrabMode::None).ok();
                                window.set_cursor_visible(true);
                            }
                        } else {
                            // Handle inventory slot selection and block actions (only on press)
                            if is_pressed {
                                match keycode {
                                    VirtualKeyCode::Key1 => inventory.select_slot(0),
                                    VirtualKeyCode::Key2 => inventory.select_slot(1),
                                    VirtualKeyCode::Key3 => inventory.select_slot(2),
                                    VirtualKeyCode::Key4 => inventory.select_slot(3),
                                    VirtualKeyCode::Key5 => inventory.select_slot(4),
                                    VirtualKeyCode::Key6 => inventory.select_slot(5),
                                    VirtualKeyCode::E => {
                                        // Place block - get selected block from inventory
                                        if let Some(block_type) = inventory.get_selected_block() {
                                            if let Some(placement_pos) = camera.get_block_placement_position(&world, 5.0) {
                                                let (x, y, z) = placement_pos;
                                                if world.place_block(x, y, z, block_type) {
                                                    println!("Placed {:?} block at ({}, {}, {})", block_type, x, y, z);
                                                }
                                            }
                                        }
                                    },
                                    VirtualKeyCode::R => {
                                        // Destroy block - target the block we're looking at
                                        if let Some((x, y, z)) = targeted_block {
                                            if world.destroy_block(x, y, z) {
                                                println!("Destroyed block at ({}, {}, {})", x, y, z);
                                                renderer.start_arm_swing();
                                            }
                                        }
                                    },
                                    _ => {}
                                }
                            }
                            // Always pass keyboard events to camera for movement
                            camera.process_keyboard(keycode, is_pressed);
                        }
                    }
                }
                WindowEvent::Resized(physical_size) => {
                    renderer.resize(physical_size);
                    camera.resize(&renderer.config);
                }
                WindowEvent::ScaleFactorChanged { new_inner_size, .. } => {
                    renderer.resize(*new_inner_size);
                    camera.resize(&renderer.config);
                }
                _ => {}
            },
            Event::MainEventsCleared => {
                let now = std::time::Instant::now();
                let dt = (now - last_frame).as_secs_f32();
                last_frame = now;
                
                // Update world chunks based on camera position
                world.update_loaded_chunks(camera.position);
                camera.update(dt, &world);
                
                // Get targeted block for highlighting
                targeted_block = camera.get_targeted_block(&world, 5.0);
                
                window.request_redraw();
            }
            Event::RedrawRequested(_) => {
                renderer.render(&camera, &mut world, &inventory, targeted_block);
            }
            Event::DeviceEvent { event: DeviceEvent::MouseMotion { delta }, .. } => {
                if mouse_captured {
                    camera.process_mouse(delta.0 as f32, delta.1 as f32);
                }
            }
            _ => {}
        }
    });
}
