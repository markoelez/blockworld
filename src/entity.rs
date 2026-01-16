use cgmath::{Point3, Vector3};
use rand::Rng;

use crate::world::{World, BlockType};

// Villager constants
pub const VILLAGER_HEIGHT: f32 = 1.8;
pub const VILLAGER_WIDTH: f32 = 0.6;
pub const VILLAGER_SPEED: f32 = 1.5;
const GRAVITY: f32 = 32.0;
const TERMINAL_VELOCITY: f32 = 50.0;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum VillagerState {
    Idle,
    Walking,
    LookingAround,
}

pub struct Villager {
    pub id: u32,
    pub position: Point3<f32>,
    pub velocity: Vector3<f32>,
    pub yaw: f32,                    // Facing direction in degrees
    pub state: VillagerState,
    pub animation_time: f32,
    pub home_chunk: (i32, i32),
    pub on_ground: bool,
    pub robe_color: f32,             // Block type for robe (18-23)

    // State timers
    state_timer: f32,
    idle_timer: f32,
    walk_timer: f32,
    look_timer: f32,
}

// Robe color options (block types 18-23)
const ROBE_COLORS: [f32; 6] = [18.0, 19.0, 20.0, 21.0, 22.0, 23.0];

impl Villager {
    pub fn new(id: u32, position: Point3<f32>, home_chunk: (i32, i32)) -> Self {
        let mut rng = rand::thread_rng();
        Self {
            id,
            position,
            velocity: Vector3::new(0.0, 0.0, 0.0),
            yaw: rng.gen_range(0.0..360.0),
            state: VillagerState::Idle,
            animation_time: 0.0,
            home_chunk,
            on_ground: false,
            robe_color: ROBE_COLORS[rng.gen_range(0..ROBE_COLORS.len())],
            state_timer: 0.0,
            idle_timer: 0.5,  // Start moving quickly
            walk_timer: 0.0,
            look_timer: 0.0,
        }
    }

    pub fn update(&mut self, dt: f32, world: &World) {
        self.animation_time += dt;

        // Apply physics
        self.update_physics(dt, world);
    }

    fn update_physics(&mut self, dt: f32, world: &World) {
        // Apply gravity
        self.velocity.y -= GRAVITY * dt;
        self.velocity.y = self.velocity.y.max(-TERMINAL_VELOCITY).min(TERMINAL_VELOCITY);

        // Apply horizontal movement based on state
        if self.state == VillagerState::Walking {
            let yaw_rad = self.yaw.to_radians();
            self.velocity.x = -yaw_rad.sin() * VILLAGER_SPEED;
            self.velocity.z = -yaw_rad.cos() * VILLAGER_SPEED;
        } else {
            self.velocity.x *= 0.8; // Friction
            self.velocity.z *= 0.8;
        }

        // Apply velocity with collision detection
        self.apply_collision(dt, world);
    }

    fn apply_collision(&mut self, dt: f32, world: &World) {
        let half_width = VILLAGER_WIDTH / 2.0;

        // X-axis movement
        let new_x = self.position.x + self.velocity.x * dt;
        let mut can_move_x = true;

        for dy in 0..2 {
            let check_y = (self.position.y - VILLAGER_HEIGHT + 0.1 + dy as f32).floor() as i32;
            for dz in [-1.0, 0.0, 1.0] {
                let check_z = (self.position.z + dz * half_width).floor() as i32;
                let check_x = if self.velocity.x > 0.0 {
                    (new_x + half_width).floor() as i32
                } else {
                    (new_x - half_width).floor() as i32
                };

                if let Some(block) = world.get_block(check_x, check_y, check_z) {
                    if block != BlockType::Air && block != BlockType::Water {
                        can_move_x = false;
                        break;
                    }
                }
            }
            if !can_move_x { break; }
        }

        if can_move_x {
            self.position.x = new_x;
        } else {
            self.velocity.x = 0.0;
        }

        // Z-axis movement
        let new_z = self.position.z + self.velocity.z * dt;
        let mut can_move_z = true;

        for dy in 0..2 {
            let check_y = (self.position.y - VILLAGER_HEIGHT + 0.1 + dy as f32).floor() as i32;
            for dx in [-1.0, 0.0, 1.0] {
                let check_x = (self.position.x + dx * half_width).floor() as i32;
                let check_z = if self.velocity.z > 0.0 {
                    (new_z + half_width).floor() as i32
                } else {
                    (new_z - half_width).floor() as i32
                };

                if let Some(block) = world.get_block(check_x, check_y, check_z) {
                    if block != BlockType::Air && block != BlockType::Water {
                        can_move_z = false;
                        break;
                    }
                }
            }
            if !can_move_z { break; }
        }

        if can_move_z {
            self.position.z = new_z;
        } else {
            self.velocity.z = 0.0;
        }

        // Y-axis movement (gravity/ground)
        let new_y = self.position.y + self.velocity.y * dt;
        self.on_ground = false;

        if self.velocity.y <= 0.0 {
            // Check ground collision
            let feet_y = (new_y - VILLAGER_HEIGHT).floor() as i32;
            for dx in [-1.0, 0.0, 1.0] {
                for dz in [-1.0, 0.0, 1.0] {
                    let check_x = (self.position.x + dx * half_width * 0.8).floor() as i32;
                    let check_z = (self.position.z + dz * half_width * 0.8).floor() as i32;

                    if let Some(block) = world.get_block(check_x, feet_y, check_z) {
                        if block != BlockType::Air && block != BlockType::Water {
                            self.position.y = (feet_y + 1) as f32 + VILLAGER_HEIGHT;
                            self.velocity.y = 0.0;
                            self.on_ground = true;
                            return;
                        }
                    }
                }
            }
        }

        // Check ceiling collision
        if self.velocity.y > 0.0 {
            let head_y = new_y.floor() as i32;
            for dx in [-1.0, 0.0, 1.0] {
                for dz in [-1.0, 0.0, 1.0] {
                    let check_x = (self.position.x + dx * half_width * 0.8).floor() as i32;
                    let check_z = (self.position.z + dz * half_width * 0.8).floor() as i32;

                    if let Some(block) = world.get_block(check_x, head_y, check_z) {
                        if block != BlockType::Air && block != BlockType::Water {
                            self.velocity.y = 0.0;
                            return;
                        }
                    }
                }
            }
        }

        self.position.y = new_y;
    }

    pub fn update_ai(&mut self, dt: f32, world: &World, rng: &mut impl Rng) {
        self.state_timer -= dt;

        match self.state {
            VillagerState::Idle => {
                self.idle_timer -= dt;
                if self.idle_timer <= 0.0 {
                    // 70% chance to walk, 30% chance to look around
                    if rng.gen::<f32>() < 0.7 {
                        self.start_walking(rng);
                    } else {
                        self.start_looking_around(rng);
                    }
                }
            }
            VillagerState::Walking => {
                self.walk_timer -= dt;

                // Check for obstacles, cliffs, or water ahead
                if self.is_blocked(world) || self.is_cliff_ahead(world) || self.is_water_ahead(world) {
                    // Turn around
                    self.yaw += 90.0 + rng.gen::<f32>() * 90.0;
                    if self.yaw >= 360.0 { self.yaw -= 360.0; }
                    self.state = VillagerState::Idle;
                    self.idle_timer = 0.5 + rng.gen::<f32>() * 1.0;
                }

                if self.walk_timer <= 0.0 {
                    self.state = VillagerState::Idle;
                    self.idle_timer = 1.0 + rng.gen::<f32>() * 2.0;
                }
            }
            VillagerState::LookingAround => {
                self.look_timer -= dt;
                // Slowly rotate
                self.yaw += 45.0 * dt;
                if self.yaw >= 360.0 { self.yaw -= 360.0; }

                if self.look_timer <= 0.0 {
                    self.state = VillagerState::Idle;
                    self.idle_timer = 1.0 + rng.gen::<f32>() * 2.0;
                }
            }
        }
    }

    fn start_walking(&mut self, rng: &mut impl Rng) {
        self.state = VillagerState::Walking;
        self.walk_timer = 3.0 + rng.gen::<f32>() * 5.0;
        // Pick a random direction
        self.yaw = rng.gen::<f32>() * 360.0;
    }

    fn start_looking_around(&mut self, rng: &mut impl Rng) {
        self.state = VillagerState::LookingAround;
        self.look_timer = 1.0 + rng.gen::<f32>() * 2.0;
    }

    fn is_blocked(&self, world: &World) -> bool {
        // Check 1.5 blocks ahead in facing direction
        let yaw_rad = self.yaw.to_radians();
        let check_x = (self.position.x - yaw_rad.sin() * 1.2).floor() as i32;
        let check_y = (self.position.y - VILLAGER_HEIGHT + 0.5).floor() as i32;
        let check_z = (self.position.z - yaw_rad.cos() * 1.2).floor() as i32;

        // Blocked if solid block at chest height
        if let Some(block) = world.get_block(check_x, check_y, check_z) {
            if block != BlockType::Air && block != BlockType::Water {
                return true;
            }
        }

        // Also check head height
        if let Some(block) = world.get_block(check_x, check_y + 1, check_z) {
            if block != BlockType::Air && block != BlockType::Water {
                return true;
            }
        }

        false
    }

    fn is_cliff_ahead(&self, world: &World) -> bool {
        // Check if there's ground 2 blocks ahead
        let yaw_rad = self.yaw.to_radians();
        let check_x = (self.position.x - yaw_rad.sin() * 1.5).floor() as i32;
        let check_y = (self.position.y - VILLAGER_HEIGHT - 1.0).floor() as i32;
        let check_z = (self.position.z - yaw_rad.cos() * 1.5).floor() as i32;

        // Cliff if no solid ground
        if let Some(block) = world.get_block(check_x, check_y, check_z) {
            if block == BlockType::Air {
                // Check one more block down (allow 1 block drop)
                if let Some(below) = world.get_block(check_x, check_y - 1, check_z) {
                    if below == BlockType::Air {
                        return true; // More than 1 block drop - it's a cliff
                    }
                }
            }
        }

        false
    }

    fn is_water_ahead(&self, world: &World) -> bool {
        // Check for water in the path ahead
        let yaw_rad = self.yaw.to_radians();
        let check_x = (self.position.x - yaw_rad.sin() * 1.5).floor() as i32;
        let feet_y = (self.position.y - VILLAGER_HEIGHT).floor() as i32;
        let check_z = (self.position.z - yaw_rad.cos() * 1.5).floor() as i32;

        // Check at feet level and one below
        for dy in -1..=0 {
            if let Some(block) = world.get_block(check_x, feet_y + dy, check_z) {
                if block == BlockType::Water {
                    return true;
                }
            }
        }

        false
    }
}

// Dropped item that can be picked up
pub struct DroppedItem {
    pub position: Point3<f32>,
    pub velocity: Vector3<f32>,
    pub block_type: BlockType,
    pub rotation: f32,
    pub lifetime: f32,
    pub bobbing_phase: f32,
}

impl DroppedItem {
    pub fn new(position: Point3<f32>, block_type: BlockType) -> Self {
        let mut rng = rand::thread_rng();
        Self {
            position,
            velocity: Vector3::new(
                rng.gen_range(-1.5..1.5),
                rng.gen_range(4.0..6.0),  // Pop up
                rng.gen_range(-1.5..1.5),
            ),
            block_type,
            rotation: rng.gen_range(0.0..std::f32::consts::TAU),
            lifetime: 300.0,  // 5 minutes
            bobbing_phase: rng.gen_range(0.0..std::f32::consts::TAU),
        }
    }

    pub fn update(&mut self, dt: f32, world: &World) -> bool {
        self.lifetime -= dt;
        if self.lifetime <= 0.0 {
            return false;
        }

        // Apply gravity
        self.velocity.y -= 25.0 * dt;
        self.velocity.y = self.velocity.y.max(-30.0);

        // Horizontal friction
        self.velocity.x *= 0.98;
        self.velocity.z *= 0.98;

        // Try to move
        let new_x = self.position.x + self.velocity.x * dt;
        let new_y = self.position.y + self.velocity.y * dt;
        let new_z = self.position.z + self.velocity.z * dt;

        // Ground collision - simple check
        let ground_y = self.find_ground_y(world, new_x as i32, new_z as i32);
        let item_radius = 0.2;

        if new_y < ground_y + item_radius {
            self.position.y = ground_y + item_radius;
            self.velocity.y *= -0.4;  // Bounce
            self.velocity.x *= 0.6;   // Friction on ground
            self.velocity.z *= 0.6;
        } else {
            self.position.y = new_y;
        }

        self.position.x = new_x;
        self.position.z = new_z;

        // Rotate
        self.rotation += dt * 2.0;
        if self.rotation > std::f32::consts::TAU {
            self.rotation -= std::f32::consts::TAU;
        }

        true
    }

    fn find_ground_y(&self, world: &World, x: i32, z: i32) -> f32 {
        // Start from current position and search downward
        let start_y = self.position.y.floor() as i32;
        for y in (0..=start_y).rev() {
            if let Some(block) = world.get_block(x, y, z) {
                if block != BlockType::Air && block != BlockType::Water {
                    return (y + 1) as f32;
                }
            }
        }
        0.0
    }
}

pub struct EntityManager {
    pub villagers: Vec<Villager>,
    pub dropped_items: Vec<DroppedItem>,
    next_id: u32,
    rng: rand::rngs::ThreadRng,
    ai_update_timer: f32,
    spawn_check_timer: f32,
}

impl EntityManager {
    pub fn new() -> Self {
        Self {
            villagers: Vec::new(),
            dropped_items: Vec::new(),
            next_id: 0,
            rng: rand::thread_rng(),
            ai_update_timer: 0.0,
            spawn_check_timer: 0.0,
        }
    }

    /// Spawn a dropped item at a position
    pub fn spawn_dropped_item(&mut self, position: Point3<f32>, block_type: BlockType) {
        // Limit total dropped items
        if self.dropped_items.len() < 200 {
            self.dropped_items.push(DroppedItem::new(position, block_type));
        }
    }

    /// Collect dropped items near the player, returns list of collected block types
    pub fn collect_nearby_items(&mut self, player_pos: Point3<f32>) -> Vec<BlockType> {
        let mut collected = Vec::new();
        let pickup_distance_sq = 2.25;  // 1.5 block radius squared

        self.dropped_items.retain(|item| {
            let dx = item.position.x - player_pos.x;
            let dy = item.position.y - player_pos.y;
            let dz = item.position.z - player_pos.z;
            let dist_sq = dx * dx + dy * dy + dz * dz;

            if dist_sq < pickup_distance_sq {
                collected.push(item.block_type);
                false  // Remove from list
            } else {
                true   // Keep in list
            }
        });

        collected
    }

    pub fn update(&mut self, dt: f32, world: &World, player_pos: Point3<f32>) {
        // Update AI at reduced rate
        self.ai_update_timer += dt;
        let update_ai = self.ai_update_timer >= 0.1;
        let ai_dt = if update_ai {
            let elapsed = self.ai_update_timer;
            self.ai_update_timer = 0.0;
            elapsed
        } else {
            0.0
        };

        // Update each villager
        for villager in &mut self.villagers {
            // Always update physics
            villager.update(dt, world);

            // Only update AI periodically and for nearby villagers
            if update_ai {
                let dist_sq = (villager.position.x - player_pos.x).powi(2)
                    + (villager.position.z - player_pos.z).powi(2);
                if dist_sq < 100.0 * 100.0 { // Within 100 blocks
                    villager.update_ai(ai_dt, world, &mut self.rng);
                }
            }
        }

        // Update dropped items
        self.dropped_items.retain_mut(|item| item.update(dt, world));

        // Periodically check for spawning
        self.spawn_check_timer -= dt;
        if self.spawn_check_timer <= 0.0 {
            self.spawn_check_timer = 2.0; // Check every 2 seconds
            self.try_spawn_villagers(world, player_pos);
            self.cleanup_distant_villagers(player_pos);
        }
    }

    fn try_spawn_villagers(&mut self, world: &World, player_pos: Point3<f32>) {
        // Limit total villagers (increased for more life)
        if self.villagers.len() >= 50 {
            return;
        }

        // Check chunks near player for village structures
        let player_chunk_x = (player_pos.x / 16.0).floor() as i32;
        let player_chunk_z = (player_pos.z / 16.0).floor() as i32;

        for dx in -4..=4 {
            for dz in -4..=4 {
                let chunk_x = player_chunk_x + dx;
                let chunk_z = player_chunk_z + dz;
                let chunk_key = (chunk_x, chunk_z);

                // Skip if chunk not loaded
                if !world.chunks.contains_key(&chunk_key) {
                    continue;
                }

                // Skip if this chunk already has villagers
                if self.villagers.iter().any(|v| v.home_chunk == chunk_key) {
                    continue;
                }

                // Check for village structure using world's noise
                let world_x = chunk_x * 16 + 8;
                let world_z = chunk_z * 16 + 8;

                if self.is_village_location(world, world_x as f64, world_z as f64) {
                    // Spawn 1-2 villagers per chunk, spread out across the chunk
                    let count = self.rng.gen_range(1..=2);
                    for _ in 0..count {
                        // Random position within the chunk (16x16 area)
                        let offset_x = self.rng.gen_range(-8.0..8.0);
                        let offset_z = self.rng.gen_range(-8.0..8.0);
                        let try_x = world_x + offset_x as i32;
                        let try_z = world_z + offset_z as i32;

                        // Find spawn position at this random location
                        if let Some(spawn_pos) = self.find_spawn_position(world, try_x, try_z) {
                            let villager = Villager::new(self.next_id, spawn_pos, chunk_key);
                            self.next_id += 1;
                            self.villagers.push(villager);

                            if self.villagers.len() >= 50 {
                                return;
                            }
                        }
                    }
                }
            }
        }
    }

    fn is_village_location(&self, world: &World, world_x: f64, world_z: f64) -> bool {
        world.is_village_location(world_x, world_z)
    }

    fn find_spawn_position(&self, world: &World, world_x: i32, world_z: i32) -> Option<Point3<f32>> {
        // Search for valid ground position with full clearance
        for y in (30..90).rev() {
            if let Some(block) = world.get_block(world_x, y, world_z) {
                if block != BlockType::Air && block != BlockType::Water {
                    // Check for full clearance above (3 blocks for safety)
                    let above1 = world.get_block(world_x, y + 1, world_z);
                    let above2 = world.get_block(world_x, y + 2, world_z);
                    let above3 = world.get_block(world_x, y + 3, world_z);

                    if above1 == Some(BlockType::Air)
                        && above2 == Some(BlockType::Air)
                        && above3 == Some(BlockType::Air) {
                        return Some(Point3::new(
                            world_x as f32 + 0.5,
                            (y + 1) as f32 + VILLAGER_HEIGHT + 0.1,  // Slight offset above ground
                            world_z as f32 + 0.5,
                        ));
                    }
                }
            }
        }
        None
    }

    fn cleanup_distant_villagers(&mut self, player_pos: Point3<f32>) {
        // Remove villagers that are too far away
        self.villagers.retain(|v| {
            let dist_sq = (v.position.x - player_pos.x).powi(2)
                + (v.position.z - player_pos.z).powi(2);
            dist_sq < 100.0 * 100.0 // Keep within 100 blocks
        });
    }

    pub fn get_villagers(&self) -> &[Villager] {
        &self.villagers
    }

    pub fn get_dropped_items(&self) -> &[DroppedItem] {
        &self.dropped_items
    }
}
