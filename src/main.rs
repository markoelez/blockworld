use winit::{
    event::{Event, WindowEvent, ElementState, VirtualKeyCode, DeviceEvent},
    event_loop::{ControlFlow, EventLoop},
    window::{Window, WindowBuilder, CursorGrabMode},
};

mod world;
mod camera;
mod renderer;
mod ui;
mod entity;

use world::World;
use camera::Camera;
use renderer::Renderer;
use ui::Inventory;
use entity::EntityManager;

#[derive(PartialEq, Clone, Copy)]
enum LoadingStage {
    Init,
    FindSpawn,
    LoadChunks,
    GenerateMeshes,
    Done,
}

fn set_cursor_captured(window: &Window, captured: bool) {
    if captured {
        if window.set_cursor_grab(CursorGrabMode::Locked).is_err() {
            window.set_cursor_grab(CursorGrabMode::Confined).ok();
        }
        window.set_cursor_visible(false);
    } else {
        window.set_cursor_grab(CursorGrabMode::None).ok();
        window.set_cursor_visible(true);
    }
}

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
    let mut entity_manager = EntityManager::new();

    let mut last_frame = std::time::Instant::now();
    let mut mouse_captured = false;
    let mut targeted_block: Option<(i32, i32, i32)> = None;
    let mut loading_stage = LoadingStage::Init;
    let mut spawn_pos = cgmath::Point3::new(0.0f32, 60.0, 0.0);
    let mut is_loaded = false;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Poll;

        match event {
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => *control_flow = ControlFlow::Exit,
                WindowEvent::KeyboardInput { input, .. } => {
                    if !is_loaded {
                        return; // Ignore input during loading
                    }
                    if let Some(keycode) = input.virtual_keycode {
                        let is_pressed = input.state == ElementState::Pressed;

                        if is_pressed && keycode == VirtualKeyCode::Escape {
                            mouse_captured = !mouse_captured;
                            set_cursor_captured(&window, mouse_captured);
                        } else {
                            if is_pressed {
                                match keycode {
                                    VirtualKeyCode::Key1 => inventory.select_slot(0),
                                    VirtualKeyCode::Key2 => inventory.select_slot(1),
                                    VirtualKeyCode::Key3 => inventory.select_slot(2),
                                    VirtualKeyCode::Key4 => inventory.select_slot(3),
                                    VirtualKeyCode::Key5 => inventory.select_slot(4),
                                    VirtualKeyCode::Key6 => inventory.select_slot(5),
                                    VirtualKeyCode::E => {
                                        if let Some(block_type) = inventory.get_selected_block() {
                                            if let Some(placement_pos) = camera.get_block_placement_position(&world, 5.0) {
                                                let (x, y, z) = placement_pos;
                                                if world.place_block(x, y, z, block_type) {
                                                    inventory.decrement_selected();
                                                    println!("Placed {:?} block at ({}, {}, {})", block_type, x, y, z);
                                                }
                                            }
                                        }
                                    },
                                    VirtualKeyCode::R => {
                                        if let Some((x, y, z)) = targeted_block {
                                            if let Some(broken_type) = world.damage_block(x, y, z) {
                                                inventory.add_block(broken_type);
                                                println!("Destroyed block at ({}, {}, {})", x, y, z);
                                            } else {
                                                println!("Hit block at ({}, {}, {})", x, y, z);
                                            }
                                            renderer.start_arm_swing();
                                        }
                                    },
                                    _ => {}
                                }
                            }
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
                if is_loaded {
                    // Normal game loop
                    let now = std::time::Instant::now();
                    let dt = (now - last_frame).as_secs_f32();
                    last_frame = now;

                    world.update_loaded_chunks(camera.position);
                    camera.update(dt, &world);
                    entity_manager.update(dt, &world, camera.position);
                    targeted_block = camera.get_targeted_block(&world, 5.0);
                }
                window.request_redraw();
            }
            Event::RedrawRequested(_) => {
                if is_loaded {
                    renderer.render(&camera, &mut world, &inventory, targeted_block, &entity_manager);
                } else {
                    // Process loading stages
                    let (progress, message) = match loading_stage {
                        LoadingStage::Init => (0.05, "Initializing..."),
                        LoadingStage::FindSpawn => (0.15, "Finding spawn..."),
                        LoadingStage::LoadChunks => (0.4, "Generating terrain..."),
                        LoadingStage::GenerateMeshes => (0.75, "Building meshes..."),
                        LoadingStage::Done => (1.0, "Ready!"),
                    };

                    renderer.render_loading_screen(progress, message);

                    // Advance to next stage after rendering current one
                    match loading_stage {
                        LoadingStage::Init => {
                            loading_stage = LoadingStage::FindSpawn;
                        }
                        LoadingStage::FindSpawn => {
                            spawn_pos = world.find_spawn_position();
                            camera.set_spawn_position(spawn_pos);
                            loading_stage = LoadingStage::LoadChunks;
                        }
                        LoadingStage::LoadChunks => {
                            world.force_load_all_chunks(spawn_pos);
                            loading_stage = LoadingStage::GenerateMeshes;
                        }
                        LoadingStage::GenerateMeshes => {
                            renderer.force_generate_all_meshes(&mut world);
                            loading_stage = LoadingStage::Done;
                        }
                        LoadingStage::Done => {
                            is_loaded = true;
                            mouse_captured = true;
                            set_cursor_captured(&window, true);
                            last_frame = std::time::Instant::now();
                        }
                    }
                }
            }
            Event::DeviceEvent { event: DeviceEvent::MouseMotion { delta }, .. } => {
                if mouse_captured && is_loaded {
                    camera.process_mouse(delta.0 as f32, delta.1 as f32);
                }
            }
            _ => {}
        }
    });
}
