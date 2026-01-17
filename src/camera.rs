use cgmath::{Matrix4, Vector3, Point3, Deg, perspective, InnerSpace};
use winit::event::VirtualKeyCode;
use wgpu::SurfaceConfiguration;
use crate::world::{World, BlockType, TorchFace, CollisionShape, BlockFacing};

const PLAYER_HEIGHT: f32 = 1.8;
const PLAYER_WIDTH: f32 = 0.6;
const GRAVITY: f32 = 32.0;
const JUMP_VELOCITY: f32 = 10.0;
const TERMINAL_VELOCITY: f32 = 50.0;  // Max fall speed
const MAX_PHYSICS_DT: f32 = 0.016;    // Cap physics step to ~60fps equivalent

#[derive(Clone, Copy)]
pub enum HungerAction {
    Walk,
    Jump,
    Sprint,
    Attack,
}

pub struct Camera {
    pub position: Point3<f32>,
    pub yaw: f32,
    pub pitch: f32,
    pub view_proj: Matrix4<f32>,
    aspect: f32,
    fovy: f32,
    znear: f32,
    zfar: f32,

    velocity: Vector3<f32>,
    on_ground: bool,
    spawn_locked: bool, // Skip physics until first movement input

    moving_forward: bool,
    moving_backward: bool,
    moving_left: bool,
    moving_right: bool,
    jump_pressed: bool,
    bob_time: f32,

    // Footstep tracking
    distance_walked: f32,
    last_ground_block: Option<BlockType>,
    footstep_pending: bool,
    was_in_water: bool,
    was_on_ground: bool,
    just_jumped: bool,
    just_landed: bool,
    just_entered_water: bool,

    // Survival stats
    pub health: f32,           // 0.0-20.0 (10 hearts)
    pub max_health: f32,       // 20.0
    pub hunger: f32,           // 0.0-20.0 (10 drumsticks)
    pub saturation: f32,       // Hidden hunger buffer
    pub damage_cooldown: f32,  // Invincibility frames after damage
    pub is_dead: bool,         // Death state
    pub air_supply: f32,       // 0.0-10.0 (10 bubbles, 15 seconds)
    pub fall_distance: f32,    // Track fall for damage calculation
    starvation_timer: f32,     // Timer for starvation damage
    regen_timer: f32,          // Timer for health regeneration
    pub spawn_point: Point3<f32>, // Respawn location
    pub damage_flash: f32,     // Visual feedback timer for damage
    pub pending_knockback: Option<Vector3<f32>>, // Knockback to apply
}

impl Camera {
    pub fn new(config: &SurfaceConfiguration) -> Self {
        let mut camera = Self {
            position: Point3::new(0.5, 50.0, 0.5), // Temporary, will be set by set_spawn_position
            yaw: 0.0,
            pitch: 0.0,
            aspect: config.width as f32 / config.height as f32,
            fovy: 70.0,
            znear: 0.1,
            zfar: 1000.0,
            view_proj: Matrix4::from_scale(1.0),

            velocity: Vector3::new(0.0, 0.0, 0.0),
            on_ground: true,
            spawn_locked: true,

            moving_forward: false,
            moving_backward: false,
            moving_left: false,
            moving_right: false,
            jump_pressed: false,
            bob_time: 0.0,

            // Footstep tracking
            distance_walked: 0.0,
            last_ground_block: None,
            footstep_pending: false,
            was_in_water: false,
            was_on_ground: false,
            just_jumped: false,
            just_landed: false,
            just_entered_water: false,

            // Survival stats
            health: 20.0,
            max_health: 20.0,
            hunger: 20.0,
            saturation: 5.0,
            damage_cooldown: 0.0,
            is_dead: false,
            air_supply: 10.0,
            fall_distance: 0.0,
            starvation_timer: 0.0,
            regen_timer: 0.0,
            spawn_point: Point3::new(0.5, 50.0, 0.5),
            damage_flash: 0.0,
            pending_knockback: None,
        };
        camera.update_view_proj();
        camera
    }
    
    pub fn set_spawn_position(&mut self, position: Point3<f32>) {
        self.position = position;
        self.spawn_point = position;
        self.velocity = Vector3::new(0.0, 0.0, 0.0);
        self.on_ground = true;
        self.spawn_locked = true;
        self.update_view_proj();
    }
    
    pub fn resize(&mut self, config: &SurfaceConfiguration) {
        self.aspect = config.width as f32 / config.height as f32;
        self.update_view_proj();
    }
    
    pub fn process_keyboard(&mut self, key: VirtualKeyCode, pressed: bool) {
        match key {
            VirtualKeyCode::W => self.moving_forward = pressed,
            VirtualKeyCode::S => self.moving_backward = pressed,
            VirtualKeyCode::A => self.moving_left = pressed,
            VirtualKeyCode::D => self.moving_right = pressed,
            VirtualKeyCode::Space => self.jump_pressed = pressed,
            _ => {}
        }
    }
    
    pub fn process_mouse(&mut self, delta_x: f32, delta_y: f32) {
        let sensitivity = 0.15;
        self.yaw += delta_x * sensitivity;
        self.pitch -= delta_y * sensitivity;

        // Clamp pitch to prevent camera flipping
        self.pitch = self.pitch.clamp(-89.0, 89.0);

        self.update_view_proj();
    }

    /// Check if a block at (bx, by, bz) would collide with a player checking from player_y
    /// Returns true if the block would block horizontal movement at the given Y level
    fn block_collides_horizontal(block: BlockType, by: i32, player_y: f32) -> bool {
        match block.collision_shape() {
            CollisionShape::None => false,
            CollisionShape::FullBlock => true,
            CollisionShape::SlabBottom => {
                // Bottom slab: collides at y range [by, by+0.5]
                let feet_y = player_y - PLAYER_HEIGHT;
                feet_y < (by as f32 + 0.5)
            }
            CollisionShape::SlabTop => {
                // Top slab: collides at y range [by+0.5, by+1.0]
                let feet_y = player_y - PLAYER_HEIGHT;
                feet_y >= (by as f32 + 0.5) || player_y > (by as f32 + 0.5)
            }
            CollisionShape::Fence => {
                // Fence: 1.5 block height, collides if player overlaps
                let feet_y = player_y - PLAYER_HEIGHT;
                feet_y < (by as f32 + 1.5)
            }
        }
    }

    /// Check if a block would stop vertical movement (for ground collision)
    fn block_collides_ground(block: BlockType, by: i32, player_feet_y: f32) -> Option<f32> {
        match block.collision_shape() {
            CollisionShape::None => None,
            CollisionShape::FullBlock => Some(by as f32 + 1.0 + PLAYER_HEIGHT),
            CollisionShape::SlabBottom => {
                // Bottom slab top surface at by + 0.5
                if player_feet_y >= by as f32 && player_feet_y < by as f32 + 0.5 {
                    Some(by as f32 + 0.5 + PLAYER_HEIGHT)
                } else if player_feet_y < by as f32 {
                    Some(by as f32 + 0.5 + PLAYER_HEIGHT)
                } else {
                    None
                }
            }
            CollisionShape::SlabTop => {
                // Top slab top surface at by + 1.0
                Some(by as f32 + 1.0 + PLAYER_HEIGHT)
            }
            CollisionShape::Fence => {
                // Fence acts as full block for ground
                Some(by as f32 + 1.0 + PLAYER_HEIGHT)
            }
        }
    }

    // Sound event methods
    pub fn get_footstep_event(&mut self) -> Option<BlockType> {
        if self.footstep_pending {
            self.footstep_pending = false;
            self.last_ground_block
        } else {
            None
        }
    }

    pub fn check_jump_event(&mut self) -> bool {
        std::mem::take(&mut self.just_jumped)
    }

    pub fn check_land_event(&mut self) -> bool {
        std::mem::take(&mut self.just_landed)
    }

    pub fn check_water_enter_event(&mut self) -> bool {
        std::mem::take(&mut self.just_entered_water)
    }

    // Survival methods
    pub fn take_damage(&mut self, amount: f32, knockback: Option<Vector3<f32>>) -> bool {
        if self.is_dead || self.damage_cooldown > 0.0 {
            return false;
        }

        self.health = (self.health - amount).max(0.0);
        self.damage_cooldown = 0.5; // 0.5 seconds invincibility
        self.damage_flash = 0.3;    // Visual feedback duration

        // Apply knockback
        if let Some(kb) = knockback {
            self.pending_knockback = Some(kb);
        }

        // Hunger cost for taking damage
        self.hunger = (self.hunger - amount * 0.1).max(0.0);

        if self.health <= 0.0 {
            self.is_dead = true;
        }

        true
    }

    pub fn heal(&mut self, amount: f32) {
        if self.is_dead {
            return;
        }
        self.health = (self.health + amount).min(self.max_health);
    }

    pub fn eat_food(&mut self, hunger_restore: f32, saturation_restore: f32) -> bool {
        if self.is_dead || self.hunger >= 20.0 {
            return false;
        }

        self.hunger = (self.hunger + hunger_restore).min(20.0);
        self.saturation = (self.saturation + saturation_restore).min(self.hunger);
        true
    }

    pub fn is_invulnerable(&self) -> bool {
        self.damage_cooldown > 0.0 || self.is_dead
    }

    pub fn respawn(&mut self) {
        self.position = self.spawn_point;
        self.velocity = Vector3::new(0.0, 0.0, 0.0);
        self.health = self.max_health;
        self.hunger = 20.0;
        self.saturation = 5.0;
        self.air_supply = 10.0;
        self.is_dead = false;
        self.damage_cooldown = 0.0;
        self.damage_flash = 0.0;
        self.fall_distance = 0.0;
        self.starvation_timer = 0.0;
        self.regen_timer = 0.0;
        self.on_ground = true;
        self.pending_knockback = None;
        self.update_view_proj();
    }

    pub fn check_damage_flash(&mut self) -> bool {
        self.damage_flash > 0.0
    }

    pub fn is_in_lava(&self, world: &World) -> bool {
        let check_ys = [
            self.position.y - PLAYER_HEIGHT,
            self.position.y - PLAYER_HEIGHT / 2.0,
            self.position.y,
        ];
        let check_offsets = [-0.3, 0.0, 0.3];

        for &check_y in &check_ys {
            let block_y = check_y.floor() as i32;
            for &dx in &check_offsets {
                for &dz in &check_offsets {
                    let check_x = (self.position.x + dx).floor() as i32;
                    let check_z = (self.position.z + dz).floor() as i32;
                    if let Some(block) = world.get_block(check_x, block_y, check_z) {
                        if block == BlockType::Lava {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    pub fn update_survival(&mut self, dt: f32, world: &World) {
        if self.is_dead {
            return;
        }

        // Update damage cooldown
        if self.damage_cooldown > 0.0 {
            self.damage_cooldown = (self.damage_cooldown - dt).max(0.0);
        }

        // Update damage flash
        if self.damage_flash > 0.0 {
            self.damage_flash = (self.damage_flash - dt).max(0.0);
        }

        // Apply pending knockback
        if let Some(kb) = self.pending_knockback.take() {
            self.velocity.x += kb.x;
            self.velocity.y += kb.y.max(4.0); // Minimum upward knockback
            self.velocity.z += kb.z;
        }

        // Lava damage (4 damage per second)
        if self.is_in_lava(world) {
            self.take_damage(4.0 * dt, None);
        }

        // Drowning - update air supply
        if self.is_underwater(world) {
            self.air_supply -= dt / 1.5; // Lose 1 bubble per 1.5 seconds
            if self.air_supply <= 0.0 {
                self.air_supply = 0.0;
                self.take_damage(2.0 * dt, None); // Drowning damage
            }
        } else {
            self.air_supply = 10.0; // Refill instantly when above water
        }

        // Hunger system
        self.update_hunger(dt);
    }

    fn update_hunger(&mut self, dt: f32) {
        // Natural depletion (very slow - about 1 hunger per 5 minutes)
        if self.saturation > 0.0 {
            self.saturation = (self.saturation - 0.001 * dt).max(0.0);
        } else {
            self.hunger = (self.hunger - 0.001 * dt).max(0.0);
        }

        // Starvation damage when hunger is 0
        if self.hunger <= 0.0 {
            self.starvation_timer += dt;
            if self.starvation_timer >= 4.0 {
                self.take_damage(1.0, None);
                self.starvation_timer = 0.0;
            }
        } else {
            self.starvation_timer = 0.0;
        }

        // Health regeneration when well-fed (hunger >= 18)
        if self.hunger >= 18.0 && self.health < self.max_health {
            self.regen_timer += dt;
            if self.regen_timer >= 0.5 {
                self.heal(1.0);
                // Costs hunger to regenerate
                if self.saturation > 0.0 {
                    self.saturation = (self.saturation - 1.5).max(0.0);
                } else {
                    self.hunger = (self.hunger - 1.5).max(0.0);
                }
                self.regen_timer = 0.0;
            }
        } else {
            self.regen_timer = 0.0;
        }
    }

    pub fn can_sprint(&self) -> bool {
        self.hunger >= 6.0 // Need at least 3 drumsticks to sprint
    }

    pub fn deplete_hunger(&mut self, action: HungerAction) {
        let cost = match action {
            HungerAction::Walk => 0.01,
            HungerAction::Jump => 0.05,
            HungerAction::Sprint => 0.03,
            HungerAction::Attack => 0.1,
        };

        if self.saturation > 0.0 {
            self.saturation = (self.saturation - cost).max(0.0);
        } else {
            self.hunger = (self.hunger - cost).max(0.0);
        }
    }

    pub fn is_underwater(&self, world: &World) -> bool {
        let head_y = self.position.y.floor() as i32;
        world.get_block(self.position.x as i32, head_y, self.position.z as i32)
            == Some(BlockType::Water)
    }

    fn has_movement_input(&self) -> bool {
        self.moving_forward || self.moving_backward || self.moving_left || self.moving_right || self.jump_pressed
    }

    /// Check if player is currently on a ladder
    fn is_on_ladder(&self, world: &World) -> bool {
        // Check if player's body overlaps with a ladder block
        let check_positions = [
            (self.position.x, self.position.y - PLAYER_HEIGHT * 0.5),  // Lower body
            (self.position.x, self.position.y - PLAYER_HEIGHT * 0.25), // Mid body
            (self.position.x, self.position.y),                        // Upper body
        ];

        for (px, py) in check_positions {
            let block_x = px.floor() as i32;
            let block_y = py.floor() as i32;
            let block_z = self.position.z.floor() as i32;

            if let Some(block) = world.get_block(block_x, block_y, block_z) {
                if block == BlockType::Ladder {
                    return true;
                }
            }
        }
        false
    }

    pub fn update(&mut self, dt: f32, world: &World) {
        // Unlock spawn lock when player tries to move
        if self.spawn_locked {
            if self.has_movement_input() {
                self.spawn_locked = false;
            } else {
                self.update_view_proj();
                return;
            }
        }

        // Sub-step physics if dt is too large to prevent tunneling through blocks
        let mut remaining_dt = dt;
        while remaining_dt > 0.0 {
            let step_dt = remaining_dt.min(MAX_PHYSICS_DT);
            self.physics_step(step_dt, world);
            remaining_dt -= step_dt;
        }

        self.update_view_proj();
    }

    fn physics_step(&mut self, dt: f32, world: &World) {
        self.bob_time += dt;
        let yaw_rad = self.yaw.to_radians();
        
        // Calculate movement direction (ignore pitch for horizontal movement)
        let front = Vector3::new(
            yaw_rad.cos(),
            0.0,
            yaw_rad.sin(),
        );
        
        let right = Vector3::new(-yaw_rad.sin(), 0.0, yaw_rad.cos());
        
        // Apply horizontal movement
        let mut move_dir = Vector3::new(0.0, 0.0, 0.0);
        
        if self.moving_forward {
            move_dir += front;
        }
        if self.moving_backward {
            move_dir -= front;
        }
        if self.moving_right {
            move_dir += right;
        }
        if self.moving_left {
            move_dir -= right;
        }
        
        // Determine if in water
        let mut is_in_water = false;
        let check_ys = [self.position.y - PLAYER_HEIGHT, self.position.y - PLAYER_HEIGHT / 2.0, self.position.y];
        let check_offsets = [-0.3, 0.0, 0.3];
        
        'water_check: for &check_y in &check_ys {
            let block_y = check_y.floor() as i32;
            for &dx in &check_offsets {
                for &dz in &check_offsets {
                    let check_x = (self.position.x + dx).floor() as i32;
                    let check_z = (self.position.z + dz).floor() as i32;
                    if let Some(block) = world.get_block(check_x, block_y, check_z) {
                        if block == BlockType::Water {
                            is_in_water = true;
                            break 'water_check;
                        }
                    }
                }
            }
        }
        
        // Check if on ladder
        let is_on_ladder = self.is_on_ladder(world);

        let move_speed = if is_in_water { 2.0 } else if is_on_ladder { 2.5 } else { 4.3 };
        let mut horizontal_velocity = Vector3::new(0.0, 0.0, 0.0);
        if move_dir.magnitude() > 0.0 {
            horizontal_velocity = move_dir.normalize() * move_speed;
        }

        // Ladder climbing
        if is_on_ladder {
            // On ladder: disable gravity, W/S to climb
            self.velocity.y = 0.0;

            if self.moving_forward {
                self.velocity.y = 4.0;  // Climb up
            } else if self.moving_backward {
                self.velocity.y = -4.0; // Climb down
            }

            // Space to jump off ladder
            if self.jump_pressed {
                self.velocity.y = JUMP_VELOCITY * 0.8;
                // Also add horizontal velocity away from ladder
                horizontal_velocity = horizontal_velocity * 1.5;
            }
        } else {
            // Normal gravity
            self.velocity.y -= GRAVITY * dt;

            // Clamp to terminal velocity
            self.velocity.y = self.velocity.y.clamp(-TERMINAL_VELOCITY, TERMINAL_VELOCITY);

            if is_in_water {
                self.velocity.y += GRAVITY * 0.9 * dt;
                self.velocity.y *= 0.95;
                self.velocity.y += (self.bob_time * 3.0).sin() * 0.05;
            }

            // Jump if on ground or in water
            if self.jump_pressed && (self.on_ground || is_in_water) {
                self.velocity.y = if is_in_water { 5.0 } else { JUMP_VELOCITY };
            }
        }

        // Apply movement with collision detection
        let mut new_position = self.position;
        
        // Check X movement collision
        let test_x = self.position.x + horizontal_velocity.x * dt;
        let mut can_move_x = true;

        for dy in 0..3 { // Check collision for player height (3 levels to catch fences)
            let check_y = (self.position.y - PLAYER_HEIGHT + dy as f32 * 0.6).floor() as i32;
            for dz in [-1, 0, 1] { // Check around player width
                let check_z = (self.position.z + dz as f32 * PLAYER_WIDTH * 0.5).floor() as i32;
                let check_x = if horizontal_velocity.x > 0.0 {
                    (test_x + PLAYER_WIDTH * 0.5).floor() as i32
                } else {
                    (test_x - PLAYER_WIDTH * 0.5).floor() as i32
                };

                if let Some(block) = world.get_block(check_x, check_y, check_z) {
                    if block != BlockType::Water && Self::block_collides_horizontal(block, check_y, self.position.y) {
                        can_move_x = false;
                        break;
                    }
                }
            }
            if !can_move_x { break; }
        }

        if can_move_x {
            new_position.x = test_x;
        }

        // Check Z movement collision
        let test_z = self.position.z + horizontal_velocity.z * dt;
        let mut can_move_z = true;

        for dy in 0..3 { // Check collision for player height (3 levels to catch fences)
            let check_y = (self.position.y - PLAYER_HEIGHT + dy as f32 * 0.6).floor() as i32;
            for dx in [-1, 0, 1] { // Check around player width
                let check_x = (new_position.x + dx as f32 * PLAYER_WIDTH * 0.5).floor() as i32;
                let check_z = if horizontal_velocity.z > 0.0 {
                    (test_z + PLAYER_WIDTH * 0.5).floor() as i32
                } else {
                    (test_z - PLAYER_WIDTH * 0.5).floor() as i32
                };

                if let Some(block) = world.get_block(check_x, check_y, check_z) {
                    if block != BlockType::Water && Self::block_collides_horizontal(block, check_y, self.position.y) {
                        can_move_z = false;
                        break;
                    }
                }
            }
            if !can_move_z { break; }
        }

        if can_move_z {
            new_position.z = test_z;
        }

        // Apply Y movement and check vertical collisions
        new_position.y += self.velocity.y * dt;

        // Check ground collision
        self.on_ground = false;
        let feet_y = new_position.y - PLAYER_HEIGHT;
        let check_block_y = feet_y.floor() as i32;

        for dx in [-1, 0, 1] {
            for dz in [-1, 0, 1] {
                let check_x = (new_position.x + dx as f32 * PLAYER_WIDTH * 0.3).floor() as i32;
                let check_z = (new_position.z + dz as f32 * PLAYER_WIDTH * 0.3).floor() as i32;

                if let Some(block) = world.get_block(check_x, check_block_y, check_z) {
                    if block != BlockType::Water {
                        if let Some(new_y) = Self::block_collides_ground(block, check_block_y, feet_y) {
                            if self.velocity.y <= 0.0 {
                                new_position.y = new_y;
                                self.velocity.y = 0.0;
                                self.on_ground = true;
                            }
                        }
                    }
                }
            }
        }

        // Check ceiling collision
        let head_y = (new_position.y + 0.1).floor() as i32;
        for dx in [-1, 0, 1] {
            for dz in [-1, 0, 1] {
                let check_x = (new_position.x + dx as f32 * PLAYER_WIDTH * 0.3).floor() as i32;
                let check_z = (new_position.z + dz as f32 * PLAYER_WIDTH * 0.3).floor() as i32;

                if let Some(block) = world.get_block(check_x, head_y, check_z) {
                    if block != BlockType::Water && block.collision_shape() != CollisionShape::None && self.velocity.y > 0.0 {
                        self.velocity.y = 0.0;
                    }
                }
            }
        }
        
        // Check if fell below world and respawn safely
        if new_position.y < -10.0 {
            // Find a safe respawn position using improved logic
            let mut respawn_found = false;
            'respawn_search: for search_radius in 0i32..15 {
                for search_x in -search_radius..=search_radius {
                    for search_z in -search_radius..=search_radius {
                        if search_radius > 0 && 
                           search_x.abs() < search_radius && 
                           search_z.abs() < search_radius {
                            continue;
                        }
                        
                        // Find highest solid block with clearance above
                        for y in (10..World::CHUNK_HEIGHT as i32).rev() {
                            if let Some(block) = world.get_block(search_x, y, search_z) {
                                if block != BlockType::Air && 
                                   block != BlockType::Barrier &&
                                   block != BlockType::Wood &&
                                   block != BlockType::Leaves {
                                    // Check for 4 blocks of clearance above
                                    let mut has_clearance = true;
                                    for check_y in 1..=4 {
                                        if let Some(above) = world.get_block(search_x, y + check_y, search_z) {
                                            if above != BlockType::Air && above != BlockType::Leaves {
                                                has_clearance = false;
                                                break;
                                            }
                                        }
                                    }
                                    
                                    if has_clearance {
                                        new_position = Point3::new(
                                            search_x as f32 + 0.5,
                                            y as f32 + 2.5,
                                            search_z as f32 + 0.5
                                        );
                                        self.velocity.y = 0.0;
                                        respawn_found = true;
                                        break 'respawn_search;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // Fallback respawn if no safe spot found - spawn high up in air
            if !respawn_found {
                new_position = Point3::new(0.5, 60.0, 0.5);
                self.velocity.y = 0.0;
            }
        }

        // Track sound events

        // Fall damage tracking
        if !self.on_ground && !is_in_water && self.velocity.y < 0.0 {
            // Track fall distance while falling
            self.fall_distance += (-self.velocity.y * dt);
        }

        // Water entry detection
        if is_in_water && !self.was_in_water {
            self.just_entered_water = true;
            self.fall_distance = 0.0; // Water cancels fall damage
        }
        self.was_in_water = is_in_water;

        // Jump detection
        if self.jump_pressed && self.was_on_ground && !is_in_water {
            self.just_jumped = true;
        }

        // Landing detection and fall damage
        if self.on_ground && !self.was_on_ground && !is_in_water {
            self.just_landed = true;

            // Apply fall damage if fell more than 5 blocks (reduced damage)
            if self.fall_distance > 5.0 {
                let damage = ((self.fall_distance - 5.0) * 0.5).floor().max(1.0);
                self.take_damage(damage, None);
            }
            self.fall_distance = 0.0;
        }
        self.was_on_ground = self.on_ground;

        // Footstep tracking - only when walking on ground
        if self.on_ground && !is_in_water {
            let distance_moved = ((new_position.x - self.position.x).powi(2)
                + (new_position.z - self.position.z).powi(2)).sqrt();

            if distance_moved > 0.01 {
                self.distance_walked += distance_moved;

                // Get the block type under feet for footstep sound
                let feet_block_y = (new_position.y - PLAYER_HEIGHT - 0.1).floor() as i32;
                self.last_ground_block = world.get_block(
                    new_position.x.floor() as i32,
                    feet_block_y,
                    new_position.z.floor() as i32
                );

                // Trigger footstep every ~2 blocks of walking
                if self.distance_walked >= 2.0 {
                    self.distance_walked = 0.0;
                    self.footstep_pending = true;
                }
            }
        }

        self.position = new_position;
    }

    pub fn get_look_direction(&self) -> Vector3<f32> {
        let yaw_rad = self.yaw.to_radians();
        let pitch_rad = self.pitch.to_radians();
        
        Vector3::new(
            yaw_rad.cos() * pitch_rad.cos(),
            pitch_rad.sin(),
            yaw_rad.sin() * pitch_rad.cos(),
        )
    }
    
    pub fn get_targeted_block(&self, world: &World, max_distance: f32) -> Option<(i32, i32, i32)> {
        let direction = self.get_look_direction();
        let step_size = 0.1;
        let steps = (max_distance / step_size) as i32;
        
        let mut current_block = None;
        
        for i in 0..steps {
            let distance = i as f32 * step_size;
            let check_pos = self.position + direction * distance;
            
            let block_x = check_pos.x.floor() as i32;
            let block_y = check_pos.y.floor() as i32;
            let block_z = check_pos.z.floor() as i32;
            
            // Skip if we're still in the same block
            if current_block == Some((block_x, block_y, block_z)) {
                continue;
            }
            
            if let Some(block) = world.get_block(block_x, block_y, block_z) {
                if block != BlockType::Air && block != BlockType::Barrier {
                    return Some((block_x, block_y, block_z));
                }
            }
            
            current_block = Some((block_x, block_y, block_z));
        }
        
        None
    }
    
    pub fn get_block_placement_position(&self, world: &World, max_distance: f32) -> Option<(i32, i32, i32)> {
        self.get_block_placement_with_face(world, max_distance).map(|(pos, _)| pos)
    }

    pub fn get_block_placement_with_face(&self, world: &World, max_distance: f32) -> Option<((i32, i32, i32), TorchFace)> {
        let direction = self.get_look_direction();
        let step_size = 0.05; // Smaller steps for more precision
        let steps = (max_distance / step_size) as i32;

        let mut last_air_pos: Option<(i32, i32, i32)> = None;

        for i in 0..steps {
            let distance = i as f32 * step_size;
            let check_pos = self.position + direction * distance;

            let block_x = check_pos.x.floor() as i32;
            let block_y = check_pos.y.floor() as i32;
            let block_z = check_pos.z.floor() as i32;

            if let Some(block) = world.get_block(block_x, block_y, block_z) {
                if block != BlockType::Air && block != BlockType::Barrier && block != BlockType::Water {
                    // Hit a solid block, determine which face we're placing on
                    if let Some(air_pos) = last_air_pos {
                        let dx = air_pos.0 - block_x;
                        let dy = air_pos.1 - block_y;
                        let dz = air_pos.2 - block_z;

                        // Determine the face based on the difference
                        // Torch should tilt AWAY from the wall it's attached to
                        let face = if dy > 0 {
                            TorchFace::Top  // Placing on top of block
                        } else if dy < 0 {
                            TorchFace::Top  // Can't place torch on bottom, default to top
                        } else if dx > 0 {
                            TorchFace::East  // Air is to +X, torch tilts toward +X (east)
                        } else if dx < 0 {
                            TorchFace::West  // Air is to -X, torch tilts toward -X (west)
                        } else if dz > 0 {
                            TorchFace::South  // Air is to +Z, torch tilts toward +Z (south)
                        } else if dz < 0 {
                            TorchFace::North  // Air is to -Z, torch tilts toward -Z (north)
                        } else {
                            TorchFace::Top  // Fallback
                        };

                        return Some((air_pos, face));
                    }
                    return None;
                } else if block == BlockType::Air {
                    // Update last known air position
                    last_air_pos = Some((block_x, block_y, block_z));
                }
            }
        }

        None
    }

    /// Get slab placement position and whether it should be top or bottom slab
    /// Returns (placement_pos, is_top_slab, hit_block_pos, hit_block_type)
    pub fn get_slab_placement(&self, world: &World, max_distance: f32) -> Option<((i32, i32, i32), bool, (i32, i32, i32), BlockType)> {
        let direction = self.get_look_direction();
        let step_size = 0.02; // Very small steps for precise slab placement
        let steps = (max_distance / step_size) as i32;

        let mut last_air_pos: Option<(i32, i32, i32)> = None;
        let mut hit_y_frac: f32 = 0.5; // Fractional Y position where we hit

        for i in 0..steps {
            let distance = i as f32 * step_size;
            let check_pos = self.position + direction * distance;

            let block_x = check_pos.x.floor() as i32;
            let block_y = check_pos.y.floor() as i32;
            let block_z = check_pos.z.floor() as i32;

            if let Some(block) = world.get_block(block_x, block_y, block_z) {
                if block != BlockType::Air && block != BlockType::Barrier && block != BlockType::Water {
                    // Hit a solid block
                    hit_y_frac = check_pos.y - check_pos.y.floor();

                    if let Some(air_pos) = last_air_pos {
                        let dy = air_pos.1 - block_y;

                        // Determine if top or bottom slab based on hit position
                        let is_top_slab = if dy > 0 {
                            // Placing on top of a block - check where we're looking on the top face
                            false // Default to bottom slab when placing on top
                        } else if dy < 0 {
                            // Placing on bottom of a block
                            true // Default to top slab when placing underneath
                        } else {
                            // Placing on side - use Y fraction of hit point
                            hit_y_frac >= 0.5
                        };

                        return Some((air_pos, is_top_slab, (block_x, block_y, block_z), block));
                    }
                    return None;
                } else if block == BlockType::Air {
                    last_air_pos = Some((block_x, block_y, block_z));
                }
            }
        }

        None
    }
    
    pub fn get_facing_direction(&self) -> &'static str {
        // Normalize yaw to 0-360 range
        let mut yaw_deg = self.yaw % 360.0;
        if yaw_deg < 0.0 {
            yaw_deg += 360.0;
        }

        // Determine cardinal direction based on yaw
        // yaw 0 = looking toward +X (East)
        // yaw 90 = looking toward +Z (South)
        // yaw 180 = looking toward -X (West)
        // yaw 270 = looking toward -Z (North)
        if yaw_deg >= 315.0 || yaw_deg < 45.0 {
            "East (+X)"
        } else if yaw_deg >= 45.0 && yaw_deg < 135.0 {
            "South (+Z)"
        } else if yaw_deg >= 135.0 && yaw_deg < 225.0 {
            "West (-X)"
        } else {
            "North (-Z)"
        }
    }

    /// Get the player's horizontal facing direction as BlockFacing
    /// Stairs should face TOWARD the player (low side toward player)
    pub fn get_block_facing(&self) -> BlockFacing {
        let mut yaw_deg = self.yaw % 360.0;
        if yaw_deg < 0.0 {
            yaw_deg += 360.0;
        }

        // Player is looking toward this direction, so stairs should face opposite
        // (stairs low side faces the player)
        if yaw_deg >= 315.0 || yaw_deg < 45.0 {
            BlockFacing::West   // Looking East, stairs face West (toward player)
        } else if yaw_deg >= 45.0 && yaw_deg < 135.0 {
            BlockFacing::North  // Looking South, stairs face North
        } else if yaw_deg >= 135.0 && yaw_deg < 225.0 {
            BlockFacing::East   // Looking West, stairs face East
        } else {
            BlockFacing::South  // Looking North, stairs face South
        }
    }

    /// Get stair placement info: (position, facing, upside_down)
    pub fn get_stair_placement(&self, world: &World, max_distance: f32) -> Option<((i32, i32, i32), BlockFacing, bool)> {
        let direction = self.get_look_direction();
        let step_size = 0.02;
        let steps = (max_distance / step_size) as i32;

        let mut last_air_pos: Option<(i32, i32, i32)> = None;
        let mut hit_y_frac: f32 = 0.5;

        for i in 0..steps {
            let distance = i as f32 * step_size;
            let check_pos = self.position + direction * distance;

            let block_x = check_pos.x.floor() as i32;
            let block_y = check_pos.y.floor() as i32;
            let block_z = check_pos.z.floor() as i32;

            if let Some(block) = world.get_block(block_x, block_y, block_z) {
                if block != BlockType::Air && block != BlockType::Barrier && block != BlockType::Water {
                    hit_y_frac = check_pos.y - check_pos.y.floor();

                    if let Some(air_pos) = last_air_pos {
                        let dy = air_pos.1 - block_y;

                        // Upside-down if hitting bottom of block or upper half of side
                        let upside_down = if dy < 0 {
                            true  // Hitting bottom of block above
                        } else if dy > 0 {
                            false // Hitting top of block below
                        } else {
                            hit_y_frac >= 0.5 // Side hit - check Y position
                        };

                        let facing = self.get_block_facing();
                        return Some((air_pos, facing, upside_down));
                    }
                    return None;
                } else if block == BlockType::Air {
                    last_air_pos = Some((block_x, block_y, block_z));
                }
            }
        }

        None
    }

    fn update_view_proj(&mut self) {
        let yaw_rad = self.yaw.to_radians();
        let pitch_rad = self.pitch.to_radians();
        
        let front = Vector3::new(
            yaw_rad.cos() * pitch_rad.cos(),
            pitch_rad.sin(),
            yaw_rad.sin() * pitch_rad.cos(),
        );
        
        let view = Matrix4::look_at_rh(
            self.position,
            self.position + front,
            Vector3::unit_y(),
        );
        
        let proj = perspective(
            Deg(self.fovy),
            self.aspect,
            self.znear,
            self.zfar,
        );
        
        self.view_proj = OPENGL_TO_WGPU_MATRIX * proj * view;
    }
}

#[rustfmt::skip]
pub const OPENGL_TO_WGPU_MATRIX: Matrix4<f32> = Matrix4::new(
    1.0, 0.0, 0.0, 0.0,
    0.0, 1.0, 0.0, 0.0,
    0.0, 0.0, 0.5, 0.0,
    0.0, 0.0, 0.5, 1.0,
);