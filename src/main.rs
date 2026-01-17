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

use world::{World, ItemStack, Tool, ToolType, ToolMaterial};
use camera::{Camera, HungerAction};
use renderer::Renderer;
use ui::{Inventory, DebugInfo, PauseMenu, ChestUI, CraftingUI, RecipeRegistry};
use entity::EntityManager;
use particle::{ParticleSystem, WeatherState, LightningSystem};
use audio::{AudioManager, MusicManager};

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
    let mut crafting_ui = CraftingUI::new();
    let mut furnace_ui = ui::FurnaceUI::new();
    let recipe_registry = RecipeRegistry::new();
    let mut entity_manager = EntityManager::new();
    let mut particle_system = ParticleSystem::new();
    let mut weather_state = WeatherState::new();
    let mut weather_rng = rand::thread_rng();
    let mut lightning_system = LightningSystem::new();
    let audio_manager = AudioManager::new();
    let mut music_manager = MusicManager::new();

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
                            if crafting_ui.open {
                                // Close crafting UI and return items to inventory
                                let items = crafting_ui.close();
                                for item in items {
                                    inventory.add_item(item);
                                }
                                mouse_captured = true;
                                set_cursor_captured(&window, true);
                            } else if chest_ui.open {
                                // Close chest UI first
                                chest_ui.close();
                                mouse_captured = true;
                                set_cursor_captured(&window, true);
                            } else if furnace_ui.open {
                                // Close furnace UI
                                furnace_ui.close();
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
                                            let contents = world.chest_contents.entry(chest_pos).or_insert([None; 9]);
                                            if chest_ui.in_chest_section {
                                                // Take from chest to inventory
                                                if let Some((block_type, qty)) = contents[chest_ui.selected_slot] {
                                                    if inventory.add_block(block_type) {
                                                        if qty <= 1 {
                                                            contents[chest_ui.selected_slot] = None;
                                                        } else {
                                                            contents[chest_ui.selected_slot] = Some((block_type, qty - 1));
                                                        }
                                                    }
                                                }
                                            } else {
                                                // Put from inventory to chest (only blocks, not tools)
                                                if let Some(ItemStack::Block(block_type, qty)) = inventory.slots[chest_ui.selected_slot].clone() {
                                                    // Try to stack with existing item in chest
                                                    let mut stacked = false;
                                                    for slot in contents.iter_mut() {
                                                        if let Some((bt, q)) = slot {
                                                            if *bt == block_type {
                                                                *q += 1;
                                                                stacked = true;
                                                                break;
                                                            }
                                                        }
                                                    }
                                                    // If not stacked, find empty slot
                                                    if !stacked {
                                                        for slot in contents.iter_mut() {
                                                            if slot.is_none() {
                                                                *slot = Some((block_type, 1));
                                                                stacked = true;
                                                                break;
                                                            }
                                                        }
                                                    }
                                                    // Only decrement inventory if transfer succeeded
                                                    if stacked {
                                                        if qty <= 1 {
                                                            inventory.slots[chest_ui.selected_slot] = None;
                                                        } else {
                                                            inventory.slots[chest_ui.selected_slot] = Some(ItemStack::Block(block_type, qty - 1));
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    },
                                    _ => {}
                                }
                            }
                        } else if crafting_ui.open {
                            // Handle crafting UI navigation
                            if is_pressed {
                                match keycode {
                                    VirtualKeyCode::Up | VirtualKeyCode::W => crafting_ui.navigate(0, -1),
                                    VirtualKeyCode::Down | VirtualKeyCode::S => crafting_ui.navigate(0, 1),
                                    VirtualKeyCode::Left | VirtualKeyCode::A => crafting_ui.navigate(-1, 0),
                                    VirtualKeyCode::Right | VirtualKeyCode::D => crafting_ui.navigate(1, 0),
                                    VirtualKeyCode::Tab => crafting_ui.switch_section(1),
                                    VirtualKeyCode::LShift => crafting_ui.switch_section(-1),
                                    VirtualKeyCode::Return => {
                                        // Handle place/take based on current section
                                        match crafting_ui.section {
                                            0 => {
                                                // In crafting grid - place item from inventory or remove
                                                let (row, col) = (crafting_ui.selected_row, crafting_ui.selected_col);
                                                if crafting_ui.grid[row][col].is_some() {
                                                    // Remove item back to inventory
                                                    if let Some(item) = crafting_ui.grid[row][col].take() {
                                                        inventory.add_item(item);
                                                    }
                                                } else if let Some(item) = inventory.slots[inventory.selected_slot].as_ref() {
                                                    // Place item from selected inventory slot
                                                    if let ItemStack::Block(block_type, qty) = item {
                                                        crafting_ui.grid[row][col] = Some(ItemStack::Block(*block_type, 1));
                                                        if *qty <= 1 {
                                                            inventory.slots[inventory.selected_slot] = None;
                                                        } else {
                                                            inventory.slots[inventory.selected_slot] = Some(ItemStack::Block(*block_type, qty - 1));
                                                        }
                                                    }
                                                }
                                            }
                                            1 => {
                                                // In result slot - take crafted item
                                                if let Some(recipe) = recipe_registry.find_match(&crafting_ui.grid, crafting_ui.grid_size) {
                                                    let result = recipe.result.clone();
                                                    if inventory.add_item(result) {
                                                        // Consume ingredients
                                                        for row in 0..crafting_ui.grid_size {
                                                            for col in 0..crafting_ui.grid_size {
                                                                if let Some(ref mut item) = crafting_ui.grid[row][col] {
                                                                    if let ItemStack::Block(_, ref mut qty) = item {
                                                                        if *qty <= 1 {
                                                                            crafting_ui.grid[row][col] = None;
                                                                        } else {
                                                                            *qty -= 1;
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            2 => {
                                                // In inventory - select slot (already handled by number keys)
                                                inventory.select_slot(crafting_ui.inventory_slot);
                                            }
                                            _ => {}
                                        }
                                    },
                                    VirtualKeyCode::Key1 => { crafting_ui.section = 2; crafting_ui.inventory_slot = 0; },
                                    VirtualKeyCode::Key2 => { crafting_ui.section = 2; crafting_ui.inventory_slot = 1; },
                                    VirtualKeyCode::Key3 => { crafting_ui.section = 2; crafting_ui.inventory_slot = 2; },
                                    VirtualKeyCode::Key4 => { crafting_ui.section = 2; crafting_ui.inventory_slot = 3; },
                                    VirtualKeyCode::Key5 => { crafting_ui.section = 2; crafting_ui.inventory_slot = 4; },
                                    VirtualKeyCode::Key6 => { crafting_ui.section = 2; crafting_ui.inventory_slot = 5; },
                                    _ => {}
                                }
                            }
                        } else if furnace_ui.open {
                            // Handle furnace UI navigation
                            if is_pressed {
                                match keycode {
                                    VirtualKeyCode::Up | VirtualKeyCode::W => furnace_ui.navigate(0, -1),
                                    VirtualKeyCode::Down | VirtualKeyCode::S => furnace_ui.navigate(0, 1),
                                    VirtualKeyCode::Left | VirtualKeyCode::A => furnace_ui.navigate(-1, 0),
                                    VirtualKeyCode::Right | VirtualKeyCode::D => furnace_ui.navigate(1, 0),
                                    VirtualKeyCode::Return => {
                                        // Transfer items between furnace slots and inventory
                                        if let Some(furnace_pos) = furnace_ui.furnace_pos {
                                            if let Some(furnace_data) = world.get_furnace_mut(furnace_pos.0, furnace_pos.1, furnace_pos.2) {
                                                match furnace_ui.selected_section {
                                                    0 => {
                                                        // Input slot - place smeltable item or take back
                                                        if furnace_data.input.is_some() {
                                                            // Take item back
                                                            if let Some((bt, qty)) = furnace_data.input.take() {
                                                                for _ in 0..qty {
                                                                    inventory.add_block(bt);
                                                                }
                                                            }
                                                        } else if let Some(block_type) = inventory.get_selected_block() {
                                                            // Check if smeltable
                                                            if world::FurnaceData::smelting_recipe(block_type).is_some() {
                                                                furnace_data.input = Some((block_type, 1));
                                                                inventory.decrement_selected();
                                                            }
                                                        }
                                                    }
                                                    1 => {
                                                        // Fuel slot - place fuel or take back
                                                        if furnace_data.fuel.is_some() {
                                                            // Take fuel back
                                                            if let Some((bt, qty)) = furnace_data.fuel.take() {
                                                                for _ in 0..qty {
                                                                    inventory.add_block(bt);
                                                                }
                                                            }
                                                        } else if let Some(block_type) = inventory.get_selected_block() {
                                                            // Check if fuel
                                                            if world::FurnaceData::fuel_burn_time(block_type).is_some() {
                                                                furnace_data.fuel = Some((block_type, 1));
                                                                inventory.decrement_selected();
                                                            }
                                                        }
                                                    }
                                                    2 => {
                                                        // Output slot - take smelted items
                                                        if let Some((bt, qty)) = furnace_data.output.take() {
                                                            for _ in 0..qty {
                                                                inventory.add_block(bt);
                                                            }
                                                        }
                                                    }
                                                    3 => {
                                                        // Inventory section - select hotbar slot
                                                        inventory.select_slot(furnace_ui.selected_slot);
                                                    }
                                                    _ => {}
                                                }
                                            }
                                        }
                                    }
                                    VirtualKeyCode::Key1 => { furnace_ui.selected_section = 3; furnace_ui.selected_slot = 0; }
                                    VirtualKeyCode::Key2 => { furnace_ui.selected_section = 3; furnace_ui.selected_slot = 1; }
                                    VirtualKeyCode::Key3 => { furnace_ui.selected_section = 3; furnace_ui.selected_slot = 2; }
                                    VirtualKeyCode::Key4 => { furnace_ui.selected_section = 3; furnace_ui.selected_slot = 3; }
                                    VirtualKeyCode::Key5 => { furnace_ui.selected_section = 3; furnace_ui.selected_slot = 4; }
                                    VirtualKeyCode::Key6 => { furnace_ui.selected_section = 3; furnace_ui.selected_slot = 5; }
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
                                        // First check if holding food - eat it
                                        if let Some(block_type) = inventory.get_selected_block() {
                                            if let Some((hunger_restore, saturation_restore)) = block_type.food_properties() {
                                                if camera.eat_food(hunger_restore, saturation_restore) {
                                                    inventory.decrement_selected();
                                                }
                                                // Don't process further if we tried to eat
                                            } else if let Some((x, y, z)) = targeted_block {
                                                // Check for interactable blocks first
                                                let target_block = world.get_block(x, y, z);
                                                if target_block == Some(world::BlockType::CraftingTable) {
                                                    // Open crafting table UI
                                                    crafting_ui.open_crafting_table((x, y, z));
                                                    mouse_captured = false;
                                                    set_cursor_captured(&window, false);
                                                } else if target_block == Some(world::BlockType::Chest) {
                                                    // Open chest UI
                                                    chest_ui.open_chest((x, y, z));
                                                    mouse_captured = false;
                                                    set_cursor_captured(&window, false);
                                                } else if target_block == Some(world::BlockType::DoorBottom)
                                                       || target_block == Some(world::BlockType::DoorTop) {
                                                    // Toggle door open/closed
                                                    if let Some(now_open) = world.toggle_door(x, y, z) {
                                                        if let Some(ref audio) = audio_manager {
                                                            if now_open {
                                                                audio.play_block_break(world::BlockType::Planks);
                                                            } else {
                                                                audio.play_block_place(world::BlockType::Planks);
                                                            }
                                                        }
                                                    }
                                                } else if target_block == Some(world::BlockType::WoodTrapdoor)
                                                       || target_block == Some(world::BlockType::IronTrapdoor) {
                                                    // Toggle trapdoor open/closed
                                                    if world.toggle_trapdoor(x, y, z) {
                                                        if let Some(ref audio) = audio_manager {
                                                            audio.play_block_place(world::BlockType::Planks);
                                                        }
                                                    }
                                                } else if target_block == Some(world::BlockType::Bed) {
                                                    // Try to sleep (only at night)
                                                    let time_of_day = renderer.get_time_of_day();
                                                    let is_night = time_of_day < 0.25 || time_of_day > 0.75;
                                                    if is_night {
                                                        // Skip to morning
                                                        renderer.set_time_of_day(0.3);
                                                        // TODO: Could set spawn point here
                                                    }
                                                } else if target_block == Some(world::BlockType::Furnace)
                                                       || target_block == Some(world::BlockType::FurnaceLit) {
                                                    // Open furnace UI
                                                    furnace_ui.open_furnace((x, y, z));
                                                    mouse_captured = false;
                                                    set_cursor_captured(&window, false);
                                                } else if block_type.is_bottom_slab() || block_type.is_top_slab() {
                                                    // Slab placement with top/bottom detection
                                                    if let Some((pos, is_top, hit_pos, hit_block)) = camera.get_slab_placement(&world, 5.0) {
                                                        let (x, y, z) = pos;

                                                        // Check if we can combine two slabs into a full block
                                                        if let Some(existing) = world.get_block(x, y, z) {
                                                            if existing.is_bottom_slab() && block_type.is_bottom_slab() {
                                                                // Placing bottom slab on air that has bottom slab? This shouldn't happen
                                                                // But if target block is a matching slab, combine them
                                                            }
                                                        }

                                                        // Check if target block is a matching slab we can combine with
                                                        if hit_block.is_bottom_slab() && block_type.to_bottom_slab() == hit_block.to_bottom_slab() {
                                                            // Combine into full block
                                                            if let Some(full_block) = hit_block.slab_to_full_block() {
                                                                let (hx, hy, hz) = hit_pos;
                                                                world.set_block(hx, hy, hz, full_block);
                                                                inventory.decrement_selected();
                                                                if let Some(ref audio) = audio_manager {
                                                                    audio.play_block_place(full_block);
                                                                }
                                                            }
                                                        } else if hit_block.is_top_slab() && block_type.to_top_slab() == hit_block.to_top_slab() {
                                                            // Combine into full block
                                                            if let Some(full_block) = hit_block.slab_to_full_block() {
                                                                let (hx, hy, hz) = hit_pos;
                                                                world.set_block(hx, hy, hz, full_block);
                                                                inventory.decrement_selected();
                                                                if let Some(ref audio) = audio_manager {
                                                                    audio.play_block_place(full_block);
                                                                }
                                                            }
                                                        } else {
                                                            // Place the appropriate slab variant
                                                            let slab_type = if is_top {
                                                                block_type.to_top_slab().unwrap_or(block_type)
                                                            } else {
                                                                block_type.to_bottom_slab().unwrap_or(block_type)
                                                            };

                                                            if world.place_block(x, y, z, slab_type) {
                                                                inventory.decrement_selected();
                                                                if let Some(ref audio) = audio_manager {
                                                                    audio.play_block_place(slab_type);
                                                                }
                                                            }
                                                        }
                                                    }
                                                } else if block_type.is_stairs() {
                                                    // Stairs need facing direction and upside-down detection
                                                    if let Some((pos, facing, upside_down)) = camera.get_stair_placement(&world, 5.0) {
                                                        let (x, y, z) = pos;
                                                        if world.place_stairs(x, y, z, block_type, facing, upside_down) {
                                                            inventory.decrement_selected();
                                                            if let Some(ref audio) = audio_manager {
                                                                audio.play_block_place(block_type);
                                                            }
                                                        }
                                                    }
                                                } else if block_type == world::BlockType::Ladder {
                                                    // Ladders need wall placement
                                                    if let Some((pos, face)) = camera.get_block_placement_with_face(&world, 5.0) {
                                                        let (x, y, z) = pos;
                                                        if world.place_ladder(x, y, z, face) {
                                                            inventory.decrement_selected();
                                                            if let Some(ref audio) = audio_manager {
                                                                audio.play_block_place(block_type);
                                                            }
                                                        }
                                                    }
                                                } else if block_type.is_trapdoor() {
                                                    // Trapdoors need top/bottom detection
                                                    if let Some((pos, is_top, _, _)) = camera.get_slab_placement(&world, 5.0) {
                                                        let (x, y, z) = pos;
                                                        let facing = camera.get_block_facing();
                                                        if world.place_trapdoor(x, y, z, block_type, facing, is_top) {
                                                            inventory.decrement_selected();
                                                            if let Some(ref audio) = audio_manager {
                                                                audio.play_block_place(block_type);
                                                            }
                                                        }
                                                    }
                                                } else if block_type == world::BlockType::Torch {
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
                                            } else {
                                                // No targeted block, just try to place
                                                if block_type.is_bottom_slab() || block_type.is_top_slab() {
                                                    // Slab placement
                                                    if let Some((pos, is_top, hit_pos, hit_block)) = camera.get_slab_placement(&world, 5.0) {
                                                        let (x, y, z) = pos;

                                                        // Check if target block is a matching slab we can combine with
                                                        if hit_block.is_bottom_slab() && block_type.to_bottom_slab() == hit_block.to_bottom_slab() {
                                                            if let Some(full_block) = hit_block.slab_to_full_block() {
                                                                let (hx, hy, hz) = hit_pos;
                                                                world.set_block(hx, hy, hz, full_block);
                                                                inventory.decrement_selected();
                                                                if let Some(ref audio) = audio_manager {
                                                                    audio.play_block_place(full_block);
                                                                }
                                                            }
                                                        } else if hit_block.is_top_slab() && block_type.to_top_slab() == hit_block.to_top_slab() {
                                                            if let Some(full_block) = hit_block.slab_to_full_block() {
                                                                let (hx, hy, hz) = hit_pos;
                                                                world.set_block(hx, hy, hz, full_block);
                                                                inventory.decrement_selected();
                                                                if let Some(ref audio) = audio_manager {
                                                                    audio.play_block_place(full_block);
                                                                }
                                                            }
                                                        } else {
                                                            let slab_type = if is_top {
                                                                block_type.to_top_slab().unwrap_or(block_type)
                                                            } else {
                                                                block_type.to_bottom_slab().unwrap_or(block_type)
                                                            };
                                                            if world.place_block(x, y, z, slab_type) {
                                                                inventory.decrement_selected();
                                                                if let Some(ref audio) = audio_manager {
                                                                    audio.play_block_place(slab_type);
                                                                }
                                                            }
                                                        }
                                                    }
                                                } else if block_type.is_stairs() {
                                                    // Stair placement
                                                    if let Some((pos, facing, upside_down)) = camera.get_stair_placement(&world, 5.0) {
                                                        let (x, y, z) = pos;
                                                        if world.place_stairs(x, y, z, block_type, facing, upside_down) {
                                                            inventory.decrement_selected();
                                                            if let Some(ref audio) = audio_manager {
                                                                audio.play_block_place(block_type);
                                                            }
                                                        }
                                                    }
                                                } else if block_type == world::BlockType::Ladder {
                                                    if let Some((pos, face)) = camera.get_block_placement_with_face(&world, 5.0) {
                                                        let (x, y, z) = pos;
                                                        if world.place_ladder(x, y, z, face) {
                                                            inventory.decrement_selected();
                                                            if let Some(ref audio) = audio_manager {
                                                                audio.play_block_place(block_type);
                                                            }
                                                        }
                                                    }
                                                } else if block_type.is_trapdoor() {
                                                    // Trapdoors need top/bottom detection
                                                    if let Some((pos, is_top, _, _)) = camera.get_slab_placement(&world, 5.0) {
                                                        let (x, y, z) = pos;
                                                        let facing = camera.get_block_facing();
                                                        if world.place_trapdoor(x, y, z, block_type, facing, is_top) {
                                                            inventory.decrement_selected();
                                                            if let Some(ref audio) = audio_manager {
                                                                audio.play_block_place(block_type);
                                                            }
                                                        }
                                                    }
                                                } else if block_type == world::BlockType::Torch {
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
                                        } else if let Some((x, y, z)) = targeted_block {
                                            // No item selected but targeting a chest - open it
                                            if world.get_block(x, y, z) == Some(world::BlockType::Chest) {
                                                chest_ui.open_chest((x, y, z));
                                                mouse_captured = false;
                                                set_cursor_captured(&window, false);
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
                                    VirtualKeyCode::G => {
                                        // Give player a diamond pickaxe for testing
                                        inventory.add_tool(Tool::new(ToolType::Pickaxe, ToolMaterial::Diamond));
                                    },
                                    VirtualKeyCode::B => {
                                        // Give player a diamond sword for testing
                                        inventory.add_tool(Tool::new(ToolType::Sword, ToolMaterial::Diamond));
                                    },
                                    VirtualKeyCode::I => {
                                        // Open inventory crafting (2x2)
                                        crafting_ui.open_inventory_crafting();
                                        mouse_captured = false;
                                        set_cursor_captured(&window, false);
                                    },
                                    VirtualKeyCode::R => {
                                        // Respawn if dead
                                        if camera.is_dead {
                                            // Drop inventory at death location before respawn
                                            let death_pos = camera.position;
                                            for slot in inventory.slots.iter_mut() {
                                                if let Some(item) = slot.take() {
                                                    match item {
                                                        ItemStack::Block(block_type, qty) => {
                                                            for _ in 0..qty {
                                                                entity_manager.spawn_dropped_item(death_pos, block_type);
                                                            }
                                                        }
                                                        ItemStack::Tool(tool) => {
                                                            entity_manager.spawn_dropped_tool(death_pos, tool);
                                                        }
                                                    }
                                                }
                                            }
                                            camera.respawn();
                                        } else {
                                            // Try to attack a hostile mob first (within 4 blocks)
                                            let mut attacked_something = false;
                                            if let Some((mob_id, _dist)) = entity_manager.get_closest_hostile_mob(camera.position, 4.0) {
                                                // Calculate damage based on held tool/weapon
                                                let damage = if let Some(tool) = inventory.get_selected_tool() {
                                                    tool.attack_damage()
                                                } else {
                                                    1.0 // Fist damage
                                                };
                                                // Calculate knockback direction from player to mob
                                                if let Some(mob) = entity_manager.get_hostile_mobs().iter().find(|m| m.id == mob_id) {
                                                    let kb_dir = cgmath::Vector3::new(
                                                        (mob.position.x - camera.position.x).signum() * 6.0,
                                                        3.0,
                                                        (mob.position.z - camera.position.z).signum() * 6.0,
                                                    );
                                                    entity_manager.damage_hostile_mob(mob_id, damage, Some(kb_dir));
                                                    // Reduce weapon durability on attack
                                                    if let Some(tool) = inventory.get_selected_tool_mut() {
                                                        tool.durability = tool.durability.saturating_sub(1);
                                                        if tool.durability == 0 {
                                                            inventory.slots[inventory.selected_slot] = None;
                                                        }
                                                    }
                                                }
                                                renderer.start_arm_swing();
                                                camera.deplete_hunger(HungerAction::Attack);
                                                attacked_something = true;
                                            }

                                            // Try to attack animals if no hostile mob nearby
                                            if !attacked_something {
                                                if let Some((animal_id, _dist)) = entity_manager.get_closest_animal(camera.position, 4.0) {
                                                    // Calculate damage based on held tool/weapon
                                                    let damage = if let Some(tool) = inventory.get_selected_tool() {
                                                        tool.attack_damage()
                                                    } else {
                                                        1.0 // Fist damage
                                                    };
                                                    // Calculate knockback direction from player to animal
                                                    if let Some(animal) = entity_manager.get_animals().iter().find(|a| a.id == animal_id) {
                                                        let kb_dir = cgmath::Vector3::new(
                                                            (animal.position.x - camera.position.x).signum() * 6.0,
                                                            3.0,
                                                            (animal.position.z - camera.position.z).signum() * 6.0,
                                                        );
                                                        // Damage animal and get meat drops if it died
                                                        if let Some((death_pos, meat_type, qty)) = entity_manager.damage_animal(animal_id, damage, Some(kb_dir)) {
                                                            // Spawn meat drops
                                                            for _ in 0..qty {
                                                                entity_manager.spawn_dropped_item(death_pos, meat_type);
                                                            }
                                                        }
                                                        // Reduce weapon durability on attack
                                                        if let Some(tool) = inventory.get_selected_tool_mut() {
                                                            tool.durability = tool.durability.saturating_sub(1);
                                                            if tool.durability == 0 {
                                                                inventory.slots[inventory.selected_slot] = None;
                                                            }
                                                        }
                                                    }
                                                    renderer.start_arm_swing();
                                                    camera.deplete_hunger(HungerAction::Attack);
                                                    attacked_something = true;
                                                }
                                            }

                                            // If nothing attacked, try breaking a block
                                            if !attacked_something {
                                                if let Some((x, y, z)) = targeted_block {
                                                    // Get block type for particles before damaging
                                                    let block_type = world.get_block(x, y, z);

                                                    // Get the currently held tool (if any)
                                                    let tool_ref = inventory.get_selected_tool();
                                                    let broken_type = world.damage_block_with_tool(x, y, z, tool_ref);

                                                    if let Some(dropped_block) = broken_type {
                                                        // Block was fully destroyed and can be harvested
                                                        let block_center = cgmath::Point3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5);
                                                        particle_system.spawn_block_break(block_center, dropped_block);
                                                        // Spawn dropped item
                                                        entity_manager.spawn_dropped_item(block_center, dropped_block);
                                                        if let Some(ref audio) = audio_manager {
                                                            audio.play_block_break(dropped_block);
                                                        }
                                                        // Reduce tool durability if a tool was used
                                                        if let Some(tool) = inventory.get_selected_tool_mut() {
                                                            tool.durability = tool.durability.saturating_sub(1);
                                                            // Remove tool if broken
                                                            if tool.durability == 0 {
                                                                inventory.slots[inventory.selected_slot] = None;
                                                            }
                                                        }
                                                    } else if let Some(bt) = block_type {
                                                        // Block was just damaged (not destroyed) or destroyed but not harvestable
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
                                            }
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
                    // Process water flow updates (limit per frame for performance)
                    world.process_water_updates(50);
                    // Update furnace smelting
                    world.update_furnaces(dt);
                    camera.update(dt, &world);
                    camera.update_survival(dt, &world);
                    entity_manager.update(dt, &world, camera.position, renderer.get_time_of_day());

                    // Check for hostile mob attacks on player
                    if !camera.is_dead {
                        for (damage, knockback) in entity_manager.check_hostile_attacks(camera.position) {
                            camera.take_damage(damage, Some(knockback));
                        }
                        // Check for projectile (arrow) hits on player
                        for damage in entity_manager.check_projectile_player_collisions(camera.position) {
                            camera.take_damage(damage, None);
                        }
                    }

                    // Handle creeper explosions
                    let exploding_creepers: Vec<_> = entity_manager.get_hostile_mobs().iter()
                        .filter(|m| m.mob_type == entity::HostileMobType::Creeper && m.is_dead())
                        .map(|m| m.position)
                        .collect();
                    for explosion_pos in exploding_creepers {
                        // Create explosion in the world
                        let _destroyed = world.create_explosion(explosion_pos, entity::CREEPER_EXPLOSION_RADIUS);

                        // Damage player if within explosion radius
                        let dist = ((camera.position.x - explosion_pos.x).powi(2)
                            + (camera.position.y - explosion_pos.y).powi(2)
                            + (camera.position.z - explosion_pos.z).powi(2)).sqrt();
                        if dist < entity::CREEPER_EXPLOSION_RADIUS * 2.0 {
                            // Damage falls off with distance
                            let damage_factor = 1.0 - (dist / (entity::CREEPER_EXPLOSION_RADIUS * 2.0));
                            let damage = entity::CREEPER_EXPLOSION_DAMAGE * damage_factor;
                            let knockback = cgmath::Vector3::new(
                                (camera.position.x - explosion_pos.x).signum() * 12.0,
                                8.0,
                                (camera.position.z - explosion_pos.z).signum() * 12.0,
                            );
                            camera.take_damage(damage, Some(knockback));
                        }
                    }

                    // Collect nearby dropped items
                    for item in entity_manager.collect_nearby_items(camera.position) {
                        inventory.add_item(item);
                    }

                    weather_state.update(dt, &mut weather_rng);
                    particle_system.spawn_weather(camera.position, &weather_state, dt);

                    // Update lightning system and play thunder sounds
                    if let Some(thunder_volume) = lightning_system.update(dt, camera.position, &weather_state, &mut weather_rng) {
                        if let Some(ref audio) = audio_manager {
                            audio.play_thunder(thunder_volume);
                        }
                    }

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

                    // Update background music based on time and underwater state
                    if let Some(ref mut music) = music_manager {
                        let is_underwater = camera.is_underwater(&world);
                        music.update(dt, renderer.get_time_of_day(), is_underwater);
                    }

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
                    renderer.render(&camera, &mut world, &inventory, targeted_block, &entity_manager, &particle_system, is_underwater, &debug_info, &pause_menu, &chest_ui, &crafting_ui, &furnace_ui, &recipe_registry, &lightning_system, &weather_state);
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
