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
mod particle;
mod audio;

use world::World;
use camera::Camera;
use renderer::Renderer;
use ui::{Inventory, DebugInfo, PauseMenu, ChestUI};
use entity::EntityManager;
use particle::{ParticleSystem, WeatherState};
use audio::AudioManager;

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
    let mut debug_info = DebugInfo::new();
    let mut pause_menu = PauseMenu::new();
    let mut chest_ui = ChestUI::new();
    let mut entity_manager = EntityManager::new();
    let mut particle_system = ParticleSystem::new();
    let mut weather_state = WeatherState::new();
    let mut weather_rng = rand::thread_rng();
    let audio_manager = AudioManager::new();

    let mut last_frame = std::time::Instant::now();
    let mut mouse_captured = false;
    let mut targeted_block: Option<(i32, i32, i32)> = None;
    let mut loading_stage = LoadingStage::Init;
    let mut spawn_pos = cgmath::Point3::new(0.0f32, 60.0, 0.0);
    let mut is_loaded = false;
    let mut torch_particle_timer = 0.0f32;

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
                            if chest_ui.open {
                                // Close chest UI first
                                chest_ui.close();
                                mouse_captured = true;
                                set_cursor_captured(&window, true);
                            } else if pause_menu.visible {
                                // Close pause menu and resume game
                                pause_menu.toggle();
                                mouse_captured = true;
                                set_cursor_captured(&window, true);
                            } else {
                                // Open pause menu
                                pause_menu.toggle();
                                mouse_captured = false;
                                set_cursor_captured(&window, false);
                            }
                        } else if is_pressed && keycode == VirtualKeyCode::F3 {
                            debug_info.toggle();
                        } else if pause_menu.visible {
                            // Handle pause menu navigation
                            if is_pressed {
                                match keycode {
                                    VirtualKeyCode::Up | VirtualKeyCode::W => pause_menu.navigate(-1),
                                    VirtualKeyCode::Down | VirtualKeyCode::S => pause_menu.navigate(1),
                                    VirtualKeyCode::Return => {
                                        match pause_menu.get_selected_action() {
                                            "RESUME" => {
                                                pause_menu.toggle();
                                                mouse_captured = true;
                                                set_cursor_captured(&window, true);
                                            },
                                            "OPTIONS" => {
                                                // TODO: Options menu not implemented yet
                                            },
                                            "QUIT" => {
                                                *control_flow = ControlFlow::Exit;
                                            },
                                            _ => {}
                                        }
                                    },
                                    _ => {}
                                }
                            }
                        } else if chest_ui.open {
                            // Handle chest UI navigation
                            if is_pressed {
                                match keycode {
                                    VirtualKeyCode::Up | VirtualKeyCode::W => chest_ui.navigate(0, -1),
                                    VirtualKeyCode::Down | VirtualKeyCode::S => chest_ui.navigate(0, 1),
                                    VirtualKeyCode::Left | VirtualKeyCode::A => chest_ui.navigate(-1, 0),
                                    VirtualKeyCode::Right | VirtualKeyCode::D => chest_ui.navigate(1, 0),
                                    VirtualKeyCode::Return => {
                                        // Transfer item between chest and inventory
                                        if let Some(chest_pos) = chest_ui.chest_pos {
                                            if chest_ui.in_chest_section {
                                                // Take from chest to inventory
                                                if let Some(contents) = world.chest_contents.get_mut(&chest_pos) {
                                                    if chest_ui.selected_slot < contents.len() {
                                                        let (block_type, qty) = contents[chest_ui.selected_slot];
                                                        if inventory.add_block(block_type) {
                                                            if qty <= 1 {
                                                                contents.remove(chest_ui.selected_slot);
                                                            } else {
                                                                contents[chest_ui.selected_slot].1 -= 1;
                                                            }
                                                        }
                                                    }
                                                }
                                            } else {
                                                // Put from inventory to chest
                                                if let Some((block_type, qty)) = inventory.slots[chest_ui.selected_slot] {
                                                    let contents = world.chest_contents.entry(chest_pos).or_insert_with(Vec::new);
                                                    // Try to stack with existing
                                                    let mut stacked = false;
                                                    for item in contents.iter_mut() {
                                                        if item.0 == block_type {
                                                            item.1 += 1;
                                                            stacked = true;
                                                            break;
                                                        }
                                                    }
                                                    if !stacked {
                                                        contents.push((block_type, 1));
                                                    }
                                                    // Decrement inventory
                                                    if qty <= 1 {
                                                        inventory.slots[chest_ui.selected_slot] = None;
                                                    } else {
                                                        inventory.slots[chest_ui.selected_slot] = Some((block_type, qty - 1));
                                                    }
                                                }
                                            }
                                        }
                                    },
                                    _ => {}
                                }
                            }
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
                                        // First check if we're targeting a chest to open it
                                        if let Some((x, y, z)) = targeted_block {
                                            if world.get_block(x, y, z) == Some(world::BlockType::Chest) {
                                                // Open chest UI
                                                chest_ui.open_chest((x, y, z));
                                                mouse_captured = false;
                                                set_cursor_captured(&window, false);
                                            } else if let Some(block_type) = inventory.get_selected_block() {
                                                // Place block
                                                if block_type == world::BlockType::Torch {
                                                    // Torches need special placement with face orientation
                                                    if let Some((pos, face)) = camera.get_block_placement_with_face(&world, 5.0) {
                                                        let (x, y, z) = pos;
                                                        if world.place_torch(x, y, z, face) {
                                                            inventory.decrement_selected();
                                                            if let Some(ref audio) = audio_manager {
                                                                audio.play_block_place(block_type);
                                                            }
                                                        }
                                                    }
                                                } else {
                                                    // Regular block placement
                                                    if let Some(placement_pos) = camera.get_block_placement_position(&world, 5.0) {
                                                        let (x, y, z) = placement_pos;
                                                        if world.place_block(x, y, z, block_type) {
                                                            inventory.decrement_selected();
                                                            if let Some(ref audio) = audio_manager {
                                                                audio.play_block_place(block_type);
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        } else if let Some(block_type) = inventory.get_selected_block() {
                                            // No targeted block, just try to place
                                            if block_type == world::BlockType::Torch {
                                                if let Some((pos, face)) = camera.get_block_placement_with_face(&world, 5.0) {
                                                    let (x, y, z) = pos;
                                                    if world.place_torch(x, y, z, face) {
                                                        inventory.decrement_selected();
                                                        if let Some(ref audio) = audio_manager {
                                                            audio.play_block_place(block_type);
                                                        }
                                                    }
                                                }
                                            } else {
                                                if let Some(placement_pos) = camera.get_block_placement_position(&world, 5.0) {
                                                    let (x, y, z) = placement_pos;
                                                    if world.place_block(x, y, z, block_type) {
                                                        inventory.decrement_selected();
                                                        if let Some(ref audio) = audio_manager {
                                                            audio.play_block_place(block_type);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    },
                                    VirtualKeyCode::T => {
                                        // Give player a torch for testing
                                        inventory.add_block(world::BlockType::Torch);
                                    },
                                    VirtualKeyCode::C => {
                                        // Give player a chest for testing
                                        inventory.add_block(world::BlockType::Chest);
                                    },
                                    VirtualKeyCode::R => {
                                        if let Some((x, y, z)) = targeted_block {
                                            // Get block type for particles before damaging
                                            let block_type = world.get_block(x, y, z);
                                            if let Some(broken_type) = world.damage_block(x, y, z) {
                                                // Block was fully destroyed - spawn more particles
                                                let block_center = cgmath::Point3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5);
                                                particle_system.spawn_block_break(block_center, broken_type);
                                                // Spawn dropped item instead of directly adding to inventory
                                                entity_manager.spawn_dropped_item(block_center, broken_type);
                                                if let Some(ref audio) = audio_manager {
                                                    audio.play_block_break(broken_type);
                                                }
                                            } else if let Some(bt) = block_type {
                                                // Block was just damaged - spawn fewer particles
                                                particle_system.spawn_block_break(
                                                    cgmath::Point3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5),
                                                    bt
                                                );
                                                if let Some(ref audio) = audio_manager {
                                                    audio.play_block_break(bt);
                                                }
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
                if is_loaded && !pause_menu.visible {
                    // Normal game loop (skip when paused)
                    let now = std::time::Instant::now();
                    let dt = (now - last_frame).as_secs_f32();
                    last_frame = now;

                    world.update_loaded_chunks(camera.position);
                    camera.update(dt, &world);
                    entity_manager.update(dt, &world, camera.position);

                    // Collect nearby dropped items
                    for block_type in entity_manager.collect_nearby_items(camera.position) {
                        inventory.add_block(block_type);
                    }

                    weather_state.update(dt, &mut weather_rng);
                    particle_system.spawn_weather(camera.position, &weather_state, dt);

                    // Spawn torch flame particles (throttled to every ~0.1 seconds)
                    torch_particle_timer += dt;
                    if torch_particle_timer >= 0.1 {
                        torch_particle_timer = 0.0;
                        let torch_positions: Vec<cgmath::Point3<f32>> = world.torch_orientations
                            .keys()
                            .filter(|(x, y, z)| {
                                // Only process torches within 30 blocks of camera
                                let dx = *x as f32 - camera.position.x;
                                let dy = *y as f32 - camera.position.y;
                                let dz = *z as f32 - camera.position.z;
                                dx * dx + dy * dy + dz * dz < 900.0
                            })
                            .map(|(x, y, z)| cgmath::Point3::new(*x as f32 + 0.5, *y as f32 + 0.5, *z as f32 + 0.5))
                            .collect();
                        particle_system.spawn_torch_flames(&torch_positions);
                    }

                    particle_system.update(dt);
                    targeted_block = camera.get_targeted_block(&world, 5.0);

                    // Update block preview for placement visualization
                    let preview_pos = match (pause_menu.visible || chest_ui.open, inventory.get_selected_block()) {
                        (true, _) | (_, None) => None,
                        (_, Some(world::BlockType::Torch)) => {
                            camera.get_block_placement_with_face(&world, 5.0).map(|(pos, _)| pos)
                        }
                        (_, Some(_)) => camera.get_block_placement_position(&world, 5.0),
                    };
                    renderer.update_block_preview(preview_pos, inventory.get_selected_block());

                    // Handle sound and particle events
                    if let Some(ref audio) = audio_manager {
                        if let Some(block_type) = camera.get_footstep_event() {
                            audio.play_footstep(block_type);
                        }
                        if camera.check_jump_event() {
                            audio.play_jump();
                        }
                        if camera.check_land_event() {
                            audio.play_land();
                        }
                    }
                    if camera.check_water_enter_event() {
                        if let Some(ref audio) = audio_manager {
                            audio.play_splash();
                        }
                        particle_system.spawn_water_splash(camera.position);
                    }
                } else if is_loaded {
                    // When paused, just update time tracking
                    last_frame = std::time::Instant::now();
                }
                window.request_redraw();
            }
            Event::RedrawRequested(_) => {
                if is_loaded {
                    let is_underwater = camera.is_underwater(&world);
                    renderer.render(&camera, &mut world, &inventory, targeted_block, &entity_manager, &particle_system, is_underwater, &debug_info, &pause_menu, &chest_ui);
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
