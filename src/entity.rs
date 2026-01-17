use cgmath::{Point3, Vector3};
use rand::Rng;

use crate::world::{World, BlockType, ItemStack, Tool};

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

// Animal constants
pub const ANIMAL_GRAVITY: f32 = 32.0;
pub const ANIMAL_TERMINAL_VELOCITY: f32 = 50.0;
pub const MAX_ANIMALS: usize = 200;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum AnimalType {
    // Existing farm animals
    Pig,
    Cow,
    Sheep,
    // Land animals
    Chicken,
    Rabbit,
    Horse,
    // Predators
    Wolf,
    Fox,
    // Aquatic
    Fish,
    Squid,
    Dolphin,
    // Flying
    Bee,
    Parrot,
    Bat,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum MovementType {
    Ground,
    Aquatic,
    Flying,
}

impl AnimalType {
    /// Returns (width, height) for collision
    pub fn dimensions(&self) -> (f32, f32) {
        match self {
            AnimalType::Pig => (0.9, 0.9),
            AnimalType::Cow => (0.9, 1.4),
            AnimalType::Sheep => (0.9, 1.2),
            AnimalType::Chicken => (0.4, 0.7),
            AnimalType::Rabbit => (0.4, 0.5),
            AnimalType::Horse => (1.4, 1.6),
            AnimalType::Wolf => (0.6, 0.85),
            AnimalType::Fox => (0.5, 0.7),
            AnimalType::Fish => (0.5, 0.3),
            AnimalType::Squid => (0.8, 0.8),
            AnimalType::Dolphin => (1.2, 0.6),
            AnimalType::Bee => (0.3, 0.3),
            AnimalType::Parrot => (0.3, 0.5),
            AnimalType::Bat => (0.4, 0.3),
        }
    }

    pub fn speed(&self) -> f32 {
        match self {
            AnimalType::Pig => 1.2,
            AnimalType::Cow => 1.0,
            AnimalType::Sheep => 1.3,
            AnimalType::Chicken => 1.0,
            AnimalType::Rabbit => 2.5,
            AnimalType::Horse => 3.0,
            AnimalType::Wolf => 2.0,
            AnimalType::Fox => 2.2,
            AnimalType::Fish => 1.5,
            AnimalType::Squid => 0.8,
            AnimalType::Dolphin => 3.0,
            AnimalType::Bee => 1.5,
            AnimalType::Parrot => 1.2,
            AnimalType::Bat => 1.8,
        }
    }

    /// Color index for rendering (block type index)
    pub fn color_index(&self) -> f32 {
        match self {
            AnimalType::Pig => 27.0,
            AnimalType::Cow => 28.0,
            AnimalType::Sheep => 29.0,
            AnimalType::Chicken => 30.0,
            AnimalType::Rabbit => 31.0,
            AnimalType::Horse => 32.0,
            AnimalType::Wolf => 33.0,
            AnimalType::Fox => 34.0,
            AnimalType::Fish => 35.0,
            AnimalType::Squid => 36.0,
            AnimalType::Dolphin => 37.0,
            AnimalType::Bee => 38.0,
            AnimalType::Parrot => 39.0,
            AnimalType::Bat => 40.0,
        }
    }

    /// Movement type for physics
    pub fn movement_type(&self) -> MovementType {
        match self {
            AnimalType::Pig | AnimalType::Cow | AnimalType::Sheep |
            AnimalType::Chicken | AnimalType::Rabbit | AnimalType::Horse |
            AnimalType::Wolf | AnimalType::Fox => MovementType::Ground,
            AnimalType::Fish | AnimalType::Squid | AnimalType::Dolphin => MovementType::Aquatic,
            AnimalType::Bee | AnimalType::Parrot | AnimalType::Bat => MovementType::Flying,
        }
    }

    /// Whether this animal is a predator
    pub fn is_predator(&self) -> bool {
        matches!(self, AnimalType::Wolf | AnimalType::Fox)
    }

    /// Whether this animal is prey (flees from predators)
    pub fn is_prey(&self) -> bool {
        matches!(self, AnimalType::Chicken | AnimalType::Rabbit | AnimalType::Sheep)
    }

    /// How many additional animals can spawn in a group (herds, flocks, schools)
    pub fn group_size(&self) -> usize {
        match self {
            // Herding animals
            AnimalType::Cow => 4,
            AnimalType::Sheep => 5,
            AnimalType::Pig => 3,
            AnimalType::Horse => 3,
            // Flocking birds
            AnimalType::Chicken => 5,
            AnimalType::Parrot => 3,
            AnimalType::Bat => 4,
            // Pack animals
            AnimalType::Wolf => 3,
            AnimalType::Fox => 2,
            // Small animals
            AnimalType::Rabbit => 4,
            // Swarms
            AnimalType::Bee => 6,
            // Schools of fish
            AnimalType::Fish => 8,
            AnimalType::Squid => 3,
            AnimalType::Dolphin => 4,
        }
    }

    /// Base health for this animal type
    pub fn base_health(&self) -> f32 {
        match self {
            AnimalType::Pig => 10.0,
            AnimalType::Cow => 10.0,
            AnimalType::Sheep => 8.0,
            AnimalType::Chicken => 4.0,
            AnimalType::Rabbit => 3.0,
            AnimalType::Horse => 15.0,
            AnimalType::Wolf => 8.0,
            AnimalType::Fox => 8.0,
            AnimalType::Fish => 3.0,
            AnimalType::Squid => 10.0,
            AnimalType::Dolphin => 10.0,
            AnimalType::Bee => 1.0,
            AnimalType::Parrot => 4.0,
            AnimalType::Bat => 3.0,
        }
    }

    /// Returns the meat drop type and quantity range for this animal
    pub fn meat_drop(&self) -> Option<(BlockType, u32, u32)> {
        match self {
            AnimalType::Pig => Some((BlockType::RawPork, 1, 3)),
            AnimalType::Cow => Some((BlockType::RawBeef, 1, 3)),
            AnimalType::Sheep => Some((BlockType::RawMutton, 1, 2)),
            AnimalType::Chicken => Some((BlockType::RawChicken, 1, 1)),
            _ => None, // Other animals don't drop meat
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum AnimalState {
    Idle,
    Walking,
    Eating,
    Running,    // Predators chasing or prey fleeing
    Swimming,   // Aquatic movement
    Flying,     // Airborne movement
    Hovering,   // Stationary in air (bees)
}

pub struct Animal {
    pub id: u32,
    pub animal_type: AnimalType,
    pub position: Point3<f32>,
    pub velocity: Vector3<f32>,
    pub yaw: f32,
    pub state: AnimalState,
    pub state_timer: f32,
    pub animation_time: f32,
    pub on_ground: bool,
    pub health: f32,
    pub max_health: f32,
    pub damage_flash: f32,
}

impl Animal {
    pub fn new(id: u32, animal_type: AnimalType, position: Point3<f32>) -> Self {
        let mut rng = rand::thread_rng();
        // Set initial state based on movement type
        let (state, velocity) = match animal_type.movement_type() {
            MovementType::Ground => (AnimalState::Idle, Vector3::new(0.0, 0.0, 0.0)),
            MovementType::Aquatic => (AnimalState::Swimming, Vector3::new(0.0, 0.0, 0.0)),
            MovementType::Flying => (AnimalState::Hovering, Vector3::new(0.0, 0.0, 0.0)),
        };
        let health = animal_type.base_health();
        Self {
            id,
            animal_type,
            position,
            velocity,
            yaw: rng.gen_range(0.0..360.0),
            state,
            state_timer: rng.gen_range(1.0..3.0),
            animation_time: 0.0,
            on_ground: false,
            health,
            max_health: health,
            damage_flash: 0.0,
        }
    }

    pub fn update(&mut self, dt: f32, world: &World) {
        self.animation_time += dt;
        if self.damage_flash > 0.0 {
            self.damage_flash = (self.damage_flash - dt).max(0.0);
        }
        self.update_physics(dt, world);
    }

    /// Take damage and return true if still alive
    pub fn take_damage(&mut self, amount: f32, knockback: Option<Vector3<f32>>) -> bool {
        self.health = (self.health - amount).max(0.0);
        self.damage_flash = 0.2;

        if let Some(kb) = knockback {
            self.velocity.x += kb.x;
            self.velocity.y += kb.y.max(4.0);
            self.velocity.z += kb.z;
        }

        // Start fleeing when damaged
        if self.health > 0.0 && self.animal_type.movement_type() == MovementType::Ground {
            self.state = AnimalState::Running;
            self.state_timer = 3.0;
        }

        self.health > 0.0
    }

    pub fn is_dead(&self) -> bool {
        self.health <= 0.0
    }

    fn update_physics(&mut self, dt: f32, world: &World) {
        match self.animal_type.movement_type() {
            MovementType::Ground => self.update_ground_physics(dt, world),
            MovementType::Aquatic => self.update_aquatic_physics(dt, world),
            MovementType::Flying => self.update_flying_physics(dt, world),
        }
    }

    fn update_ground_physics(&mut self, dt: f32, world: &World) {
        let (width, height) = self.animal_type.dimensions();
        let half_width = width / 2.0;

        // Apply gravity
        self.velocity.y -= ANIMAL_GRAVITY * dt;
        self.velocity.y = self.velocity.y.max(-ANIMAL_TERMINAL_VELOCITY).min(ANIMAL_TERMINAL_VELOCITY);

        // Apply horizontal movement based on state
        if self.state == AnimalState::Walking || self.state == AnimalState::Running {
            let speed = if self.state == AnimalState::Running {
                self.animal_type.speed() * 1.8
            } else {
                self.animal_type.speed()
            };
            let yaw_rad = self.yaw.to_radians();
            self.velocity.x = -yaw_rad.sin() * speed;
            self.velocity.z = -yaw_rad.cos() * speed;
        } else {
            self.velocity.x *= 0.8;
            self.velocity.z *= 0.8;
        }

        // X-axis collision
        let new_x = self.position.x + self.velocity.x * dt;
        let mut can_move_x = true;
        for dy in 0..2 {
            let check_y = (self.position.y - height + 0.1 + dy as f32).floor() as i32;
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

        // Z-axis collision
        let new_z = self.position.z + self.velocity.z * dt;
        let mut can_move_z = true;
        for dy in 0..2 {
            let check_y = (self.position.y - height + 0.1 + dy as f32).floor() as i32;
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

        // Y-axis (gravity/ground)
        let new_y = self.position.y + self.velocity.y * dt;
        self.on_ground = false;

        if self.velocity.y <= 0.0 {
            let feet_y = (new_y - height).floor() as i32;
            for dx in [-1.0, 0.0, 1.0] {
                for dz in [-1.0, 0.0, 1.0] {
                    let check_x = (self.position.x + dx * half_width * 0.8).floor() as i32;
                    let check_z = (self.position.z + dz * half_width * 0.8).floor() as i32;
                    if let Some(block) = world.get_block(check_x, feet_y, check_z) {
                        if block != BlockType::Air && block != BlockType::Water {
                            self.position.y = (feet_y + 1) as f32 + height;
                            self.velocity.y = 0.0;
                            self.on_ground = true;
                            return;
                        }
                    }
                }
            }
        }
        self.position.y = new_y;
    }

    fn update_aquatic_physics(&mut self, dt: f32, world: &World) {
        let (_, height) = self.animal_type.dimensions();

        // Check if currently in water
        let center_y = (self.position.y - height / 2.0).floor() as i32;
        let in_water = world.get_block(
            self.position.x.floor() as i32,
            center_y,
            self.position.z.floor() as i32
        ) == Some(BlockType::Water);

        if in_water {
            // Swimming - 3D movement, no gravity
            if self.state == AnimalState::Swimming {
                let speed = self.animal_type.speed();
                let yaw_rad = self.yaw.to_radians();
                self.velocity.x = -yaw_rad.sin() * speed;
                self.velocity.z = -yaw_rad.cos() * speed;

                // Gentle vertical movement
                self.velocity.y *= 0.95;
            } else {
                // Slow down when idle
                self.velocity.x *= 0.9;
                self.velocity.z *= 0.9;
                self.velocity.y *= 0.9;
            }

            // Apply movement
            self.position.x += self.velocity.x * dt;
            self.position.y += self.velocity.y * dt;
            self.position.z += self.velocity.z * dt;
        } else {
            // Out of water - apply gravity (flopping)
            self.velocity.y -= ANIMAL_GRAVITY * 0.5 * dt;
            self.velocity.x *= 0.95;
            self.velocity.z *= 0.95;

            self.position.x += self.velocity.x * dt;
            self.position.y += self.velocity.y * dt;
            self.position.z += self.velocity.z * dt;

            // Ground collision
            let feet_y = (self.position.y - height).floor() as i32;
            if let Some(block) = world.get_block(
                self.position.x.floor() as i32,
                feet_y,
                self.position.z.floor() as i32
            ) {
                if block != BlockType::Air && block != BlockType::Water {
                    self.position.y = (feet_y + 1) as f32 + height;
                    self.velocity.y = 0.0;
                }
            }
        }
    }

    fn update_flying_physics(&mut self, dt: f32, world: &World) {
        let (width, height) = self.animal_type.dimensions();

        // Flying - no gravity, maintain altitude
        if self.state == AnimalState::Flying {
            let speed = self.animal_type.speed();
            let yaw_rad = self.yaw.to_radians();
            self.velocity.x = -yaw_rad.sin() * speed;
            self.velocity.z = -yaw_rad.cos() * speed;
        } else if self.state == AnimalState::Hovering {
            // Hover in place with gentle bobbing
            self.velocity.x *= 0.9;
            self.velocity.z *= 0.9;
            self.velocity.y = (self.animation_time * 2.0).sin() * 0.3;
        } else {
            // Idle - slow descent
            self.velocity.x *= 0.9;
            self.velocity.z *= 0.9;
            self.velocity.y = -0.5;
        }

        // Find ground level below
        let mut ground_y = 0;
        for y in (0..(self.position.y as i32)).rev() {
            if let Some(block) = world.get_block(
                self.position.x.floor() as i32,
                y,
                self.position.z.floor() as i32
            ) {
                if block != BlockType::Air && block != BlockType::Water {
                    ground_y = y + 1;
                    break;
                }
            }
        }

        // Maintain altitude above ground (2-8 blocks)
        let target_min_y = ground_y as f32 + 2.0 + height;
        let target_max_y = ground_y as f32 + 8.0 + height;

        if self.position.y < target_min_y && (self.state == AnimalState::Flying || self.state == AnimalState::Hovering) {
            self.velocity.y += 5.0 * dt;
        } else if self.position.y > target_max_y {
            self.velocity.y -= 3.0 * dt;
        }

        // Apply movement
        let new_x = self.position.x + self.velocity.x * dt;
        let new_z = self.position.z + self.velocity.z * dt;

        // Simple collision check for flying
        let check_y = self.position.y.floor() as i32;
        let blocked_x = world.get_block(new_x.floor() as i32, check_y, self.position.z.floor() as i32)
            .map(|b| b != BlockType::Air && b != BlockType::Water)
            .unwrap_or(false);
        let blocked_z = world.get_block(self.position.x.floor() as i32, check_y, new_z.floor() as i32)
            .map(|b| b != BlockType::Air && b != BlockType::Water)
            .unwrap_or(false);

        if !blocked_x {
            self.position.x = new_x;
        } else {
            self.velocity.x = 0.0;
        }
        if !blocked_z {
            self.position.z = new_z;
        } else {
            self.velocity.z = 0.0;
        }

        self.position.y += self.velocity.y * dt;
        self.position.y = self.position.y.max(target_min_y);
    }

    /// Check if this animal is in water
    fn is_in_water(&self, world: &World) -> bool {
        let (_, height) = self.animal_type.dimensions();
        let center_y = (self.position.y - height / 2.0).floor() as i32;
        world.get_block(
            self.position.x.floor() as i32,
            center_y,
            self.position.z.floor() as i32
        ) == Some(BlockType::Water)
    }

    pub fn update_ai(&mut self, dt: f32, world: &World, rng: &mut impl Rng) {
        self.state_timer -= dt;

        match self.animal_type.movement_type() {
            MovementType::Ground => self.update_ground_ai(dt, world, rng),
            MovementType::Aquatic => self.update_aquatic_ai(dt, world, rng),
            MovementType::Flying => self.update_flying_ai(dt, world, rng),
        }
    }

    fn update_ground_ai(&mut self, _dt: f32, world: &World, rng: &mut impl Rng) {
        match self.state {
            AnimalState::Idle => {
                if self.state_timer <= 0.0 {
                    let roll: f32 = rng.gen();
                    if roll < 0.5 {
                        self.state = AnimalState::Walking;
                        self.yaw = rng.gen_range(0.0..360.0);
                        self.state_timer = rng.gen_range(3.0..8.0);
                    } else if roll < 0.8 {
                        self.state = AnimalState::Eating;
                        self.state_timer = rng.gen_range(2.0..4.0);
                    } else {
                        self.state_timer = rng.gen_range(2.0..5.0);
                    }
                }
            }
            AnimalState::Walking => {
                if self.is_blocked(world) || self.is_cliff_ahead(world) || self.is_water_ahead(world) {
                    self.yaw += 90.0 + rng.gen::<f32>() * 90.0;
                    if self.yaw >= 360.0 { self.yaw -= 360.0; }
                    self.state = AnimalState::Idle;
                    self.state_timer = rng.gen_range(1.0..3.0);
                }
                if self.state_timer <= 0.0 {
                    self.state = AnimalState::Idle;
                    self.state_timer = rng.gen_range(1.0..3.0);
                }
            }
            AnimalState::Running => {
                // Running from predator or chasing prey
                if self.state_timer <= 0.0 {
                    self.state = AnimalState::Idle;
                    self.state_timer = rng.gen_range(1.0..3.0);
                }
                if self.is_blocked(world) || self.is_cliff_ahead(world) {
                    self.yaw += 90.0 + rng.gen::<f32>() * 90.0;
                    if self.yaw >= 360.0 { self.yaw -= 360.0; }
                }
            }
            AnimalState::Eating => {
                if self.state_timer <= 0.0 {
                    self.state = AnimalState::Idle;
                    self.state_timer = rng.gen_range(1.0..3.0);
                }
            }
            _ => {
                // Invalid state for ground animal
                self.state = AnimalState::Idle;
                self.state_timer = 1.0;
            }
        }
    }

    fn update_aquatic_ai(&mut self, _dt: f32, world: &World, rng: &mut impl Rng) {
        let in_water = self.is_in_water(world);

        match self.state {
            AnimalState::Idle => {
                if self.state_timer <= 0.0 {
                    if in_water {
                        self.state = AnimalState::Swimming;
                        self.yaw = rng.gen_range(0.0..360.0);
                        // Vertical direction
                        self.velocity.y = rng.gen_range(-0.5..0.5);
                        self.state_timer = rng.gen_range(3.0..8.0);
                    } else {
                        // Try to get back to water - random flop direction
                        self.yaw = rng.gen_range(0.0..360.0);
                        self.state_timer = rng.gen_range(0.5..1.5);
                    }
                }
            }
            AnimalState::Swimming => {
                if !in_water {
                    self.state = AnimalState::Idle;
                    self.state_timer = 0.5;
                } else if self.state_timer <= 0.0 {
                    // Change direction or stop
                    if rng.gen::<f32>() < 0.3 {
                        self.state = AnimalState::Idle;
                        self.state_timer = rng.gen_range(1.0..3.0);
                    } else {
                        self.yaw = rng.gen_range(0.0..360.0);
                        self.velocity.y = rng.gen_range(-0.5..0.5);
                        self.state_timer = rng.gen_range(2.0..6.0);
                    }
                }
            }
            _ => {
                self.state = AnimalState::Idle;
                self.state_timer = 1.0;
            }
        }
    }

    fn update_flying_ai(&mut self, _dt: f32, world: &World, rng: &mut impl Rng) {
        match self.state {
            AnimalState::Idle => {
                if self.state_timer <= 0.0 {
                    let roll: f32 = rng.gen();
                    if roll < 0.4 {
                        self.state = AnimalState::Flying;
                        self.yaw = rng.gen_range(0.0..360.0);
                        self.state_timer = rng.gen_range(3.0..8.0);
                    } else if roll < 0.7 {
                        self.state = AnimalState::Hovering;
                        self.state_timer = rng.gen_range(2.0..5.0);
                    } else {
                        self.state_timer = rng.gen_range(1.0..3.0);
                    }
                }
            }
            AnimalState::Flying => {
                if self.is_blocked(world) {
                    self.yaw += 90.0 + rng.gen::<f32>() * 90.0;
                    if self.yaw >= 360.0 { self.yaw -= 360.0; }
                }
                if self.state_timer <= 0.0 {
                    self.state = AnimalState::Hovering;
                    self.state_timer = rng.gen_range(2.0..4.0);
                }
            }
            AnimalState::Hovering => {
                if self.state_timer <= 0.0 {
                    let roll: f32 = rng.gen();
                    if roll < 0.6 {
                        self.state = AnimalState::Flying;
                        self.yaw = rng.gen_range(0.0..360.0);
                        self.state_timer = rng.gen_range(3.0..8.0);
                    } else {
                        self.state = AnimalState::Idle;
                        self.state_timer = rng.gen_range(1.0..3.0);
                    }
                }
            }
            _ => {
                self.state = AnimalState::Hovering;
                self.state_timer = 1.0;
            }
        }
    }

    fn is_blocked(&self, world: &World) -> bool {
        let (_, height) = self.animal_type.dimensions();
        let yaw_rad = self.yaw.to_radians();
        let check_x = (self.position.x - yaw_rad.sin() * 1.2).floor() as i32;
        let check_y = (self.position.y - height + 0.5).floor() as i32;
        let check_z = (self.position.z - yaw_rad.cos() * 1.2).floor() as i32;

        if let Some(block) = world.get_block(check_x, check_y, check_z) {
            if block != BlockType::Air && block != BlockType::Water {
                return true;
            }
        }
        false
    }

    fn is_cliff_ahead(&self, world: &World) -> bool {
        let (_, height) = self.animal_type.dimensions();
        let yaw_rad = self.yaw.to_radians();
        let check_x = (self.position.x - yaw_rad.sin() * 1.5).floor() as i32;
        let check_y = (self.position.y - height - 1.0).floor() as i32;
        let check_z = (self.position.z - yaw_rad.cos() * 1.5).floor() as i32;

        if let Some(block) = world.get_block(check_x, check_y, check_z) {
            if block == BlockType::Air {
                if let Some(below) = world.get_block(check_x, check_y - 1, check_z) {
                    if below == BlockType::Air {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn is_water_ahead(&self, world: &World) -> bool {
        let (_, height) = self.animal_type.dimensions();
        let yaw_rad = self.yaw.to_radians();
        let check_x = (self.position.x - yaw_rad.sin() * 1.5).floor() as i32;
        let feet_y = (self.position.y - height).floor() as i32;
        let check_z = (self.position.z - yaw_rad.cos() * 1.5).floor() as i32;

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

// Hostile mob constants
pub const ZOMBIE_HEIGHT: f32 = 1.9;
pub const ZOMBIE_WIDTH: f32 = 0.6;
pub const ZOMBIE_SPEED: f32 = 2.3;
pub const ZOMBIE_DETECTION_RANGE: f32 = 40.0;
pub const ZOMBIE_ATTACK_RANGE: f32 = 2.0;
pub const ZOMBIE_DAMAGE: f32 = 3.0;
pub const ZOMBIE_HEALTH: f32 = 20.0;
pub const MAX_HOSTILE_MOBS: usize = 30;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum HostileMobType {
    Zombie,
}

impl HostileMobType {
    pub fn health(&self) -> f32 {
        match self {
            HostileMobType::Zombie => ZOMBIE_HEALTH,
        }
    }

    pub fn damage(&self) -> f32 {
        match self {
            HostileMobType::Zombie => ZOMBIE_DAMAGE,
        }
    }

    pub fn speed(&self) -> f32 {
        match self {
            HostileMobType::Zombie => ZOMBIE_SPEED,
        }
    }

    pub fn detection_range(&self) -> f32 {
        match self {
            HostileMobType::Zombie => ZOMBIE_DETECTION_RANGE,
        }
    }

    pub fn attack_range(&self) -> f32 {
        match self {
            HostileMobType::Zombie => ZOMBIE_ATTACK_RANGE,
        }
    }

    pub fn dimensions(&self) -> (f32, f32) {
        match self {
            HostileMobType::Zombie => (ZOMBIE_WIDTH, ZOMBIE_HEIGHT),
        }
    }

    /// Color index for rendering
    pub fn color_index(&self) -> f32 {
        match self {
            HostileMobType::Zombie => 48.0, // New texture index for zombies
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum HostileMobState {
    Idle,
    Wandering,
    Chasing,
    Attacking,
}

pub struct HostileMob {
    pub id: u32,
    pub mob_type: HostileMobType,
    pub position: Point3<f32>,
    pub velocity: Vector3<f32>,
    pub yaw: f32,
    pub state: HostileMobState,
    pub health: f32,
    pub max_health: f32,
    pub attack_cooldown: f32,
    pub damage_flash: f32,
    pub animation_time: f32,
    pub on_ground: bool,
    state_timer: f32,
}

impl HostileMob {
    pub fn new(id: u32, mob_type: HostileMobType, position: Point3<f32>) -> Self {
        let mut rng = rand::thread_rng();
        Self {
            id,
            mob_type,
            position,
            velocity: Vector3::new(0.0, 0.0, 0.0),
            yaw: rng.gen_range(0.0..360.0),
            state: HostileMobState::Idle,
            health: mob_type.health(),
            max_health: mob_type.health(),
            attack_cooldown: 0.0,
            damage_flash: 0.0,
            animation_time: 0.0,
            on_ground: false,
            state_timer: rng.gen_range(1.0..3.0),
        }
    }

    pub fn update(&mut self, dt: f32, world: &World) {
        self.animation_time += dt;

        // Update cooldowns
        if self.attack_cooldown > 0.0 {
            self.attack_cooldown = (self.attack_cooldown - dt).max(0.0);
        }
        if self.damage_flash > 0.0 {
            self.damage_flash = (self.damage_flash - dt).max(0.0);
        }

        self.update_physics(dt, world);
    }

    fn update_physics(&mut self, dt: f32, world: &World) {
        let (width, height) = self.mob_type.dimensions();
        let half_width = width / 2.0;

        // Apply gravity
        self.velocity.y -= GRAVITY * dt;
        self.velocity.y = self.velocity.y.max(-TERMINAL_VELOCITY).min(TERMINAL_VELOCITY);

        // Apply horizontal movement based on state
        if self.state == HostileMobState::Wandering || self.state == HostileMobState::Chasing {
            let speed = if self.state == HostileMobState::Chasing {
                self.mob_type.speed()
            } else {
                self.mob_type.speed() * 0.4 // Slower when wandering
            };
            let yaw_rad = self.yaw.to_radians();
            self.velocity.x = -yaw_rad.sin() * speed;
            self.velocity.z = -yaw_rad.cos() * speed;
        } else {
            self.velocity.x *= 0.8;
            self.velocity.z *= 0.8;
        }

        // X-axis collision
        let new_x = self.position.x + self.velocity.x * dt;
        let mut can_move_x = true;
        for dy in 0..2 {
            let check_y = (self.position.y - height + 0.1 + dy as f32).floor() as i32;
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

        // Z-axis collision
        let new_z = self.position.z + self.velocity.z * dt;
        let mut can_move_z = true;
        for dy in 0..2 {
            let check_y = (self.position.y - height + 0.1 + dy as f32).floor() as i32;
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

        // Y-axis (gravity/ground)
        let new_y = self.position.y + self.velocity.y * dt;
        self.on_ground = false;

        if self.velocity.y <= 0.0 {
            let feet_y = (new_y - height).floor() as i32;
            for dx in [-1.0, 0.0, 1.0] {
                for dz in [-1.0, 0.0, 1.0] {
                    let check_x = (self.position.x + dx * half_width * 0.8).floor() as i32;
                    let check_z = (self.position.z + dz * half_width * 0.8).floor() as i32;
                    if let Some(block) = world.get_block(check_x, feet_y, check_z) {
                        if block != BlockType::Air && block != BlockType::Water {
                            self.position.y = (feet_y + 1) as f32 + height;
                            self.velocity.y = 0.0;
                            self.on_ground = true;
                            return;
                        }
                    }
                }
            }
        }
        self.position.y = new_y;
    }

    pub fn update_ai(&mut self, dt: f32, world: &World, player_pos: Point3<f32>, rng: &mut impl Rng) {
        self.state_timer -= dt;

        let distance_to_player = ((self.position.x - player_pos.x).powi(2)
            + (self.position.y - player_pos.y).powi(2)
            + (self.position.z - player_pos.z).powi(2)).sqrt();

        let detection_range = self.mob_type.detection_range();
        let attack_range = self.mob_type.attack_range();

        match self.state {
            HostileMobState::Idle | HostileMobState::Wandering => {
                // Check if player is in detection range
                if distance_to_player < detection_range {
                    self.state = HostileMobState::Chasing;
                    self.face_player(player_pos);
                } else if self.state == HostileMobState::Idle && self.state_timer <= 0.0 {
                    // Start wandering
                    if rng.gen::<f32>() < 0.5 {
                        self.state = HostileMobState::Wandering;
                        self.yaw = rng.gen_range(0.0..360.0);
                        self.state_timer = rng.gen_range(3.0..6.0);
                    } else {
                        self.state_timer = rng.gen_range(2.0..5.0);
                    }
                } else if self.state == HostileMobState::Wandering {
                    // Check for obstacles
                    if self.is_blocked(world) || self.is_cliff_ahead(world) {
                        self.yaw += 90.0 + rng.gen::<f32>() * 90.0;
                        if self.yaw >= 360.0 { self.yaw -= 360.0; }
                    }
                    if self.state_timer <= 0.0 {
                        self.state = HostileMobState::Idle;
                        self.state_timer = rng.gen_range(1.0..3.0);
                    }
                }
            }
            HostileMobState::Chasing => {
                // Update facing direction towards player
                self.face_player(player_pos);

                // Jump if blocked
                if self.is_blocked(world) && self.on_ground {
                    self.velocity.y = 8.0; // Jump
                }

                // Check if close enough to attack
                if distance_to_player < attack_range {
                    self.state = HostileMobState::Attacking;
                } else if distance_to_player > detection_range * 1.5 {
                    // Lost player, go back to idle
                    self.state = HostileMobState::Idle;
                    self.state_timer = rng.gen_range(2.0..4.0);
                }
            }
            HostileMobState::Attacking => {
                self.face_player(player_pos);

                // Move back to chasing if player moved away
                if distance_to_player > attack_range * 1.5 {
                    self.state = HostileMobState::Chasing;
                }
            }
        }
    }

    fn face_player(&mut self, player_pos: Point3<f32>) {
        let dx = player_pos.x - self.position.x;
        let dz = player_pos.z - self.position.z;
        self.yaw = (-dx).atan2(-dz).to_degrees();
    }

    fn is_blocked(&self, world: &World) -> bool {
        let (_, height) = self.mob_type.dimensions();
        let yaw_rad = self.yaw.to_radians();
        let check_x = (self.position.x - yaw_rad.sin() * 1.2).floor() as i32;
        let check_y = (self.position.y - height + 0.5).floor() as i32;
        let check_z = (self.position.z - yaw_rad.cos() * 1.2).floor() as i32;

        if let Some(block) = world.get_block(check_x, check_y, check_z) {
            if block != BlockType::Air && block != BlockType::Water {
                return true;
            }
        }
        false
    }

    fn is_cliff_ahead(&self, world: &World) -> bool {
        let (_, height) = self.mob_type.dimensions();
        let yaw_rad = self.yaw.to_radians();
        let check_x = (self.position.x - yaw_rad.sin() * 1.5).floor() as i32;
        let check_y = (self.position.y - height - 1.0).floor() as i32;
        let check_z = (self.position.z - yaw_rad.cos() * 1.5).floor() as i32;

        if let Some(block) = world.get_block(check_x, check_y, check_z) {
            if block == BlockType::Air {
                if let Some(below) = world.get_block(check_x, check_y - 1, check_z) {
                    if below == BlockType::Air {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Take damage and return true if still alive
    pub fn take_damage(&mut self, amount: f32, knockback: Option<Vector3<f32>>) -> bool {
        self.health = (self.health - amount).max(0.0);
        self.damage_flash = 0.2;

        if let Some(kb) = knockback {
            self.velocity.x += kb.x;
            self.velocity.y += kb.y.max(4.0);
            self.velocity.z += kb.z;
        }

        self.health > 0.0
    }

    /// Check if mob can attack (attack cooldown is 0)
    pub fn can_attack(&self) -> bool {
        self.attack_cooldown <= 0.0 && self.state == HostileMobState::Attacking
    }

    /// Perform attack and reset cooldown
    pub fn perform_attack(&mut self) -> f32 {
        self.attack_cooldown = 1.0; // 1 second cooldown
        self.mob_type.damage()
    }

    pub fn is_dead(&self) -> bool {
        self.health <= 0.0
    }
}

// Dropped item that can be picked up
pub struct DroppedItem {
    pub position: Point3<f32>,
    pub velocity: Vector3<f32>,
    pub item: ItemStack,
    pub rotation: f32,
    pub lifetime: f32,
    pub bobbing_phase: f32,
}

impl DroppedItem {
    pub fn new(position: Point3<f32>, item: ItemStack) -> Self {
        let mut rng = rand::thread_rng();
        Self {
            position,
            velocity: Vector3::new(
                rng.gen_range(-1.5..1.5),
                rng.gen_range(4.0..6.0),  // Pop up
                rng.gen_range(-1.5..1.5),
            ),
            item,
            rotation: rng.gen_range(0.0..std::f32::consts::TAU),
            lifetime: 300.0,  // 5 minutes
            bobbing_phase: rng.gen_range(0.0..std::f32::consts::TAU),
        }
    }

    /// Helper to create a dropped block item
    pub fn new_block(position: Point3<f32>, block_type: BlockType) -> Self {
        Self::new(position, ItemStack::Block(block_type, 1))
    }

    /// Helper to create a dropped tool item
    pub fn new_tool(position: Point3<f32>, tool: Tool) -> Self {
        Self::new(position, ItemStack::Tool(tool))
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

        self.rotation = (self.rotation + dt * 2.0).rem_euclid(std::f32::consts::TAU);

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
    pub animals: Vec<Animal>,
    pub hostile_mobs: Vec<HostileMob>,
    next_id: u32,
    rng: rand::rngs::ThreadRng,
    ai_update_timer: f32,
    spawn_check_timer: f32,
    animal_spawn_timer: f32,
    hostile_spawn_timer: f32,
}

impl EntityManager {
    pub fn new() -> Self {
        Self {
            villagers: Vec::new(),
            dropped_items: Vec::new(),
            animals: Vec::new(),
            hostile_mobs: Vec::new(),
            next_id: 0,
            rng: rand::thread_rng(),
            ai_update_timer: 0.0,
            spawn_check_timer: 0.0,
            animal_spawn_timer: 0.0,
            hostile_spawn_timer: 0.0,
        }
    }

    /// Spawn a dropped item at a position
    pub fn spawn_dropped_item(&mut self, position: Point3<f32>, block_type: BlockType) {
        // Limit total dropped items
        if self.dropped_items.len() < 200 {
            self.dropped_items.push(DroppedItem::new_block(position, block_type));
        }
    }

    /// Spawn a dropped tool at a position
    pub fn spawn_dropped_tool(&mut self, position: Point3<f32>, tool: Tool) {
        if self.dropped_items.len() < 200 {
            self.dropped_items.push(DroppedItem::new_tool(position, tool));
        }
    }

    /// Collect dropped items near the player, returns list of collected items
    pub fn collect_nearby_items(&mut self, player_pos: Point3<f32>) -> Vec<ItemStack> {
        let mut collected = Vec::new();

        // Horizontal pickup radius (squared) - 1.5 blocks
        let horizontal_dist_sq = 2.25;
        // Vertical pickup range - player can reach items from feet to above head
        // player_pos is at eye level (~1.7 above ground), so check items from -2.0 to +1.0 relative
        let vertical_range_below = 2.0;  // Items at feet level
        let vertical_range_above = 1.0;  // Items slightly above eye level

        self.dropped_items.retain(|item| {
            let dx = item.position.x - player_pos.x;
            let dy = item.position.y - player_pos.y;
            let dz = item.position.z - player_pos.z;

            // Check horizontal distance (XZ plane)
            let horiz_dist_sq = dx * dx + dz * dz;

            // Check vertical range (items can be below feet to above head)
            let in_vertical_range = dy > -vertical_range_below && dy < vertical_range_above;

            if horiz_dist_sq < horizontal_dist_sq && in_vertical_range {
                collected.push(item.item.clone());
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

        // Update animals
        for animal in &mut self.animals {
            animal.update(dt, world);

            if update_ai {
                let dist_sq = (animal.position.x - player_pos.x).powi(2)
                    + (animal.position.z - player_pos.z).powi(2);
                if dist_sq < 80.0 * 80.0 {
                    animal.update_ai(ai_dt, world, &mut self.rng);
                }
            }
        }

        // Periodically check for spawning
        self.spawn_check_timer -= dt;
        if self.spawn_check_timer <= 0.0 {
            self.spawn_check_timer = 2.0; // Check every 2 seconds
            self.try_spawn_villagers(world, player_pos);
            self.cleanup_distant_villagers(player_pos);
        }

        // Animal spawning (separate timer)
        self.animal_spawn_timer -= dt;
        if self.animal_spawn_timer <= 0.0 {
            self.animal_spawn_timer = 3.0; // Check every 3 seconds
            self.try_spawn_animals(world, player_pos);
            self.cleanup_distant_animals(player_pos);
        }

        // Update hostile mobs
        for mob in &mut self.hostile_mobs {
            mob.update(dt, world);

            if update_ai {
                let dist_sq = (mob.position.x - player_pos.x).powi(2)
                    + (mob.position.z - player_pos.z).powi(2);
                if dist_sq < 100.0 * 100.0 {
                    mob.update_ai(ai_dt, world, player_pos, &mut self.rng);
                }
            }
        }

        // Remove dead hostile mobs
        self.hostile_mobs.retain(|mob| !mob.is_dead());

        // Hostile mob spawning
        self.hostile_spawn_timer -= dt;
        if self.hostile_spawn_timer <= 0.0 {
            self.hostile_spawn_timer = 5.0; // Check every 5 seconds
            self.try_spawn_hostile_mobs(world, player_pos);
            self.cleanup_distant_hostile_mobs(player_pos);
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

    pub fn get_animals(&self) -> &[Animal] {
        &self.animals
    }

    fn try_spawn_animals(&mut self, world: &World, player_pos: Point3<f32>) {
        if self.animals.len() >= MAX_ANIMALS {
            return;
        }

        let player_chunk_x = (player_pos.x / 16.0).floor() as i32;
        let player_chunk_z = (player_pos.z / 16.0).floor() as i32;

        // Check a wider area for spawning (6 chunk radius = ~100 block radius)
        for dx in -6..=6 {
            for dz in -6..=6 {
                let chunk_x = player_chunk_x + dx;
                let chunk_z = player_chunk_z + dz;
                let chunk_key = (chunk_x, chunk_z);

                // Skip if chunk not loaded
                if !world.chunks.contains_key(&chunk_key) {
                    continue;
                }

                // Get biome for this chunk
                let world_x = (chunk_x * 16 + 8) as f64;
                let world_z = (chunk_z * 16 + 8) as f64;
                let biome = world.get_biome(world_x, world_z);

                // Higher spawn chance per chunk (8%) for a more alive world
                if self.rng.gen::<f32>() > 0.08 {
                    continue;
                }

                // Check if this chunk already has enough animals nearby
                let chunk_center = Point3::new(
                    (chunk_x * 16 + 8) as f32,
                    player_pos.y,
                    (chunk_z * 16 + 8) as f32,
                );
                let animals_in_chunk = self.animals.iter().filter(|a| {
                    let dist_sq = (a.position.x - chunk_center.x).powi(2)
                        + (a.position.z - chunk_center.z).powi(2);
                    dist_sq < 20.0 * 20.0
                }).count();

                // Allow more animals per chunk area
                if animals_in_chunk >= 12 {
                    continue;
                }

                // Pick animal type based on biome
                use crate::world::Biome;
                let animal_type = match biome {
                    Biome::Plains => {
                        match self.rng.gen_range(0..100) {
                            0..=10 => AnimalType::Pig,
                            11..=18 => AnimalType::Cow,
                            19..=26 => AnimalType::Sheep,
                            27..=34 => AnimalType::Chicken,
                            35..=42 => AnimalType::Rabbit,
                            43..=46 => AnimalType::Horse,
                            47..=72 => AnimalType::Bee,     // 25% bees
                            73..=90 => AnimalType::Parrot,  // 18% parrots
                            _ => AnimalType::Bat,           // 9% bats
                        }
                    }
                    Biome::Forest => {
                        match self.rng.gen_range(0..100) {
                            0..=8 => AnimalType::Pig,
                            9..=14 => AnimalType::Sheep,
                            15..=22 => AnimalType::Chicken,
                            23..=30 => AnimalType::Rabbit,
                            31..=36 => AnimalType::Wolf,
                            37..=42 => AnimalType::Fox,
                            43..=68 => AnimalType::Bee,     // 25% bees
                            69..=88 => AnimalType::Parrot,  // 20% parrots
                            _ => AnimalType::Bat,           // 11% bats
                        }
                    }
                    Biome::Tundra => {
                        match self.rng.gen_range(0..100) {
                            0..=40 => AnimalType::Sheep,
                            41..=60 => AnimalType::Rabbit,
                            61..=85 => AnimalType::Wolf,
                            _ => AnimalType::Sheep,
                        }
                    }
                    Biome::Mountains => {
                        match self.rng.gen_range(0..100) {
                            0..=60 => AnimalType::Sheep,
                            61..=80 => AnimalType::Rabbit,
                            81..=95 => AnimalType::Bat,
                            _ => AnimalType::Sheep,
                        }
                    }
                    Biome::Ocean => {
                        match self.rng.gen_range(0..100) {
                            0..=50 => AnimalType::Fish,
                            51..=75 => AnimalType::Squid,
                            76..=100 => AnimalType::Dolphin,
                            _ => AnimalType::Fish,
                        }
                    }
                    Biome::Desert => {
                        // Few animals in desert
                        if self.rng.gen::<f32>() < 0.3 {
                            AnimalType::Rabbit
                        } else {
                            continue;
                        }
                    }
                };

                // Try to find spawn position
                let offset_x = self.rng.gen_range(-8..8);
                let offset_z = self.rng.gen_range(-8..8);
                let try_x = chunk_x * 16 + 8 + offset_x;
                let try_z = chunk_z * 16 + 8 + offset_z;

                // For aquatic animals, search multiple positions since water is harder to find
                let is_aquatic = animal_type.movement_type() == MovementType::Aquatic;
                let mut first_spawn_pos = None;

                if is_aquatic {
                    // Search in expanding pattern for water
                    let search_offsets = [
                        (0, 0), (4, 0), (-4, 0), (0, 4), (0, -4),
                        (8, 0), (-8, 0), (0, 8), (0, -8),
                        (4, 4), (-4, 4), (4, -4), (-4, -4),
                    ];
                    for (dx, dz) in search_offsets {
                        let search_x = try_x + dx;
                        let search_z = try_z + dz;
                        if let Some(spawn_pos) = self.find_animal_spawn_position(world, search_x, search_z, animal_type) {
                            let animal = Animal::new(self.next_id, animal_type, spawn_pos);
                            self.next_id += 1;
                            self.animals.push(animal);
                            first_spawn_pos = Some((spawn_pos, search_x, search_z));
                            break;
                        }
                    }
                } else {
                    // First try the selected animal type
                    if let Some(spawn_pos) = self.find_animal_spawn_position(world, try_x, try_z, animal_type) {
                        let animal = Animal::new(self.next_id, animal_type, spawn_pos);
                        self.next_id += 1;
                        self.animals.push(animal);
                        first_spawn_pos = Some((spawn_pos, try_x, try_z));
                    }
                }

                // Group spawning - spawn additional animals of the same type nearby
                if let Some((_, base_x, base_z)) = first_spawn_pos {
                    let group_size = animal_type.group_size();
                    let extra_to_spawn = self.rng.gen_range(0..group_size);

                    for _ in 0..extra_to_spawn {
                        if self.animals.len() >= MAX_ANIMALS {
                            return;
                        }

                        // Try nearby positions for group members
                        let offset_x = self.rng.gen_range(-4..=4);
                        let offset_z = self.rng.gen_range(-4..=4);
                        let group_x = base_x + offset_x;
                        let group_z = base_z + offset_z;

                        if let Some(group_pos) = self.find_animal_spawn_position(world, group_x, group_z, animal_type) {
                            let animal = Animal::new(self.next_id, animal_type, group_pos);
                            self.next_id += 1;
                            self.animals.push(animal);
                        }
                    }
                }

                if self.animals.len() >= MAX_ANIMALS {
                    return;
                }

                // Always also try to spawn aquatic animals if there's water nearby
                // Search a wide area for water - check in expanding circles
                let water_offsets = [
                    (0, 0), (4, 0), (-4, 0), (0, 4), (0, -4),
                    (8, 0), (-8, 0), (0, 8), (0, -8),
                    (8, 8), (-8, 8), (8, -8), (-8, -8),
                    (12, 0), (-12, 0), (0, 12), (0, -12),
                    (16, 0), (-16, 0), (0, 16), (0, -16),
                ];
                for water_offset in water_offsets {
                    let water_x = try_x + water_offset.0;
                    let water_z = try_z + water_offset.1;
                    let aquatic_type = match self.rng.gen_range(0..3) {
                        0 => AnimalType::Fish,
                        1 => AnimalType::Squid,
                        _ => AnimalType::Dolphin,
                    };
                    if let Some(spawn_pos) = self.find_animal_spawn_position(world, water_x, water_z, aquatic_type) {
                        // 70% chance to spawn aquatic when water found
                        if self.rng.gen::<f32>() < 0.7 {
                            // Spawn first aquatic animal
                            let animal = Animal::new(self.next_id, aquatic_type, spawn_pos);
                            self.next_id += 1;
                            self.animals.push(animal);

                            // Spawn a school/pod around it
                            let group_size = aquatic_type.group_size();
                            let extra_to_spawn = self.rng.gen_range(1..=group_size);
                            for _ in 0..extra_to_spawn {
                                if self.animals.len() >= MAX_ANIMALS {
                                    return;
                                }
                                let gx = water_x + self.rng.gen_range(-3..=3);
                                let gz = water_z + self.rng.gen_range(-3..=3);
                                if let Some(gpos) = self.find_animal_spawn_position(world, gx, gz, aquatic_type) {
                                    let animal = Animal::new(self.next_id, aquatic_type, gpos);
                                    self.next_id += 1;
                                    self.animals.push(animal);
                                }
                            }

                            if self.animals.len() >= MAX_ANIMALS {
                                return;
                            }
                            break; // Only spawn one group per chunk check
                        }
                    }
                }
            }
        }
    }

    fn find_animal_spawn_position(&self, world: &World, world_x: i32, world_z: i32, animal_type: AnimalType) -> Option<Point3<f32>> {
        let (_, height) = animal_type.dimensions();

        match animal_type.movement_type() {
            MovementType::Ground => {
                // Search for valid ground position
                for y in (30..90).rev() {
                    if let Some(block) = world.get_block(world_x, y, world_z) {
                        // Must spawn on grass, dirt, sand, or snow
                        if block == BlockType::Grass || block == BlockType::Dirt
                            || block == BlockType::Sand || block == BlockType::Snow {
                            let above1 = world.get_block(world_x, y + 1, world_z);
                            let above2 = world.get_block(world_x, y + 2, world_z);

                            if above1 == Some(BlockType::Air) && above2 == Some(BlockType::Air) {
                                return Some(Point3::new(
                                    world_x as f32 + 0.5,
                                    (y + 1) as f32 + height,
                                    world_z as f32 + 0.5,
                                ));
                            }
                        }
                    }
                }
                None
            }
            MovementType::Aquatic => {
                // Search for water position - search wide range
                for y in (5..70).rev() {
                    if let Some(block) = world.get_block(world_x, y, world_z) {
                        if block == BlockType::Water {
                            // Allow spawning in any water
                            return Some(Point3::new(
                                world_x as f32 + 0.5,
                                y as f32 + 0.5 + height,
                                world_z as f32 + 0.5,
                            ));
                        }
                    }
                }
                None
            }
            MovementType::Flying => {
                // Find ground level, then spawn above it
                for y in (30..90).rev() {
                    if let Some(block) = world.get_block(world_x, y, world_z) {
                        if block != BlockType::Air && block != BlockType::Water {
                            // Found ground, spawn 3-6 blocks above
                            let spawn_y = y + 4;
                            if world.get_block(world_x, spawn_y, world_z) == Some(BlockType::Air) {
                                return Some(Point3::new(
                                    world_x as f32 + 0.5,
                                    spawn_y as f32 + height,
                                    world_z as f32 + 0.5,
                                ));
                            }
                            break;
                        }
                    }
                }
                None
            }
        }
    }

    fn cleanup_distant_animals(&mut self, player_pos: Point3<f32>) {
        self.animals.retain(|a| {
            let dist_sq = (a.position.x - player_pos.x).powi(2)
                + (a.position.z - player_pos.z).powi(2);
            dist_sq < 80.0 * 80.0 // Keep within 80 blocks
        });
    }

    fn try_spawn_hostile_mobs(&mut self, world: &World, player_pos: Point3<f32>) {
        if self.hostile_mobs.len() >= MAX_HOSTILE_MOBS {
            return;
        }

        // Only spawn when it's dark (night time) - check light level by time of day
        // For now, use spawn distance from player as primary constraint

        let player_chunk_x = (player_pos.x / 16.0).floor() as i32;
        let player_chunk_z = (player_pos.z / 16.0).floor() as i32;

        // Check chunks at medium distance (2-6 chunks from player)
        for dx in -6..=6 {
            for dz in -6..=6 {
                // Skip too close to player (within 24 blocks ~ 1.5 chunks)
                let dist = ((dx * dx + dz * dz) as f32).sqrt();
                if dist < 1.5 || dist > 6.0 {
                    continue;
                }

                let chunk_x = player_chunk_x + dx;
                let chunk_z = player_chunk_z + dz;
                let chunk_key = (chunk_x, chunk_z);

                // Skip if chunk not loaded
                if !world.chunks.contains_key(&chunk_key) {
                    continue;
                }

                // Low spawn chance per chunk (2%)
                if self.rng.gen::<f32>() > 0.02 {
                    continue;
                }

                // Check if this area already has hostile mobs
                let chunk_center = Point3::new(
                    (chunk_x * 16 + 8) as f32,
                    player_pos.y,
                    (chunk_z * 16 + 8) as f32,
                );
                let mobs_in_area = self.hostile_mobs.iter().filter(|m| {
                    let dist_sq = (m.position.x - chunk_center.x).powi(2)
                        + (m.position.z - chunk_center.z).powi(2);
                    dist_sq < 30.0 * 30.0
                }).count();

                if mobs_in_area >= 3 {
                    continue;
                }

                // Try to find spawn position
                let offset_x = self.rng.gen_range(-8..8);
                let offset_z = self.rng.gen_range(-8..8);
                let try_x = chunk_x * 16 + 8 + offset_x;
                let try_z = chunk_z * 16 + 8 + offset_z;

                if let Some(spawn_pos) = self.find_hostile_spawn_position(world, try_x, try_z) {
                    // Check distance from player (must be at least 24 blocks)
                    let player_dist_sq = (spawn_pos.x - player_pos.x).powi(2)
                        + (spawn_pos.z - player_pos.z).powi(2);
                    if player_dist_sq < 24.0 * 24.0 {
                        continue;
                    }

                    let mob = HostileMob::new(self.next_id, HostileMobType::Zombie, spawn_pos);
                    self.next_id += 1;
                    self.hostile_mobs.push(mob);

                    if self.hostile_mobs.len() >= MAX_HOSTILE_MOBS {
                        return;
                    }
                }
            }
        }
    }

    fn find_hostile_spawn_position(&self, world: &World, world_x: i32, world_z: i32) -> Option<Point3<f32>> {
        // Search for valid ground position
        for y in (30..90).rev() {
            if let Some(block) = world.get_block(world_x, y, world_z) {
                // Can spawn on any solid block
                if block != BlockType::Air && block != BlockType::Water && block != BlockType::Lava {
                    let above1 = world.get_block(world_x, y + 1, world_z);
                    let above2 = world.get_block(world_x, y + 2, world_z);
                    let above3 = world.get_block(world_x, y + 3, world_z);

                    if above1 == Some(BlockType::Air)
                        && above2 == Some(BlockType::Air)
                        && above3 == Some(BlockType::Air) {
                        return Some(Point3::new(
                            world_x as f32 + 0.5,
                            (y + 1) as f32 + ZOMBIE_HEIGHT,
                            world_z as f32 + 0.5,
                        ));
                    }
                }
            }
        }
        None
    }

    fn cleanup_distant_hostile_mobs(&mut self, player_pos: Point3<f32>) {
        self.hostile_mobs.retain(|m| {
            let dist_sq = (m.position.x - player_pos.x).powi(2)
                + (m.position.z - player_pos.z).powi(2);
            dist_sq < 128.0 * 128.0 // Despawn if > 128 blocks away
        });
    }

    /// Check for hostile mob attacks on player, returns list of (damage, knockback direction)
    pub fn check_hostile_attacks(&mut self, player_pos: Point3<f32>) -> Vec<(f32, Vector3<f32>)> {
        let mut attacks = Vec::new();

        for mob in &mut self.hostile_mobs {
            if mob.can_attack() {
                let distance = ((mob.position.x - player_pos.x).powi(2)
                    + (mob.position.y - player_pos.y).powi(2)
                    + (mob.position.z - player_pos.z).powi(2)).sqrt();

                if distance < mob.mob_type.attack_range() {
                    let damage = mob.perform_attack();
                    let knockback_dir = Vector3::new(
                        (player_pos.x - mob.position.x).signum() * 8.0,
                        2.0,
                        (player_pos.z - mob.position.z).signum() * 8.0,
                    );
                    attacks.push((damage, knockback_dir));
                }
            }
        }

        attacks
    }

    /// Damage a hostile mob by ID, returns true if mob died
    pub fn damage_hostile_mob(&mut self, mob_id: u32, damage: f32, knockback: Option<Vector3<f32>>) -> bool {
        if let Some(mob) = self.hostile_mobs.iter_mut().find(|m| m.id == mob_id) {
            let survived = mob.take_damage(damage, knockback);
            !survived // Return true if mob died
        } else {
            false
        }
    }

    /// Get the closest hostile mob to a position within range, returns (mob_id, distance)
    pub fn get_closest_hostile_mob(&self, pos: Point3<f32>, max_range: f32) -> Option<(u32, f32)> {
        let mut closest: Option<(u32, f32)> = None;

        for mob in &self.hostile_mobs {
            let dist = ((mob.position.x - pos.x).powi(2)
                + (mob.position.y - pos.y).powi(2)
                + (mob.position.z - pos.z).powi(2)).sqrt();

            if dist < max_range {
                if closest.is_none() || dist < closest.unwrap().1 {
                    closest = Some((mob.id, dist));
                }
            }
        }

        closest
    }

    pub fn get_hostile_mobs(&self) -> &[HostileMob] {
        &self.hostile_mobs
    }

    /// Get the closest animal to a position within range, returns (animal_id, distance)
    pub fn get_closest_animal(&self, pos: Point3<f32>, max_range: f32) -> Option<(u32, f32)> {
        let mut closest: Option<(u32, f32)> = None;

        for animal in &self.animals {
            let dist = ((animal.position.x - pos.x).powi(2)
                + (animal.position.y - pos.y).powi(2)
                + (animal.position.z - pos.z).powi(2)).sqrt();

            if dist < max_range {
                if closest.is_none() || dist < closest.unwrap().1 {
                    closest = Some((animal.id, dist));
                }
            }
        }

        closest
    }

    /// Damage an animal by ID, returns meat drops if animal died
    pub fn damage_animal(&mut self, animal_id: u32, damage: f32, knockback: Option<Vector3<f32>>) -> Option<(Point3<f32>, BlockType, u32)> {
        if let Some(index) = self.animals.iter().position(|a| a.id == animal_id) {
            let survived = self.animals[index].take_damage(damage, knockback);
            if !survived {
                // Animal died - get meat drop before removing
                let animal = &self.animals[index];
                let death_pos = animal.position;
                let meat_drop = animal.animal_type.meat_drop();

                // Remove the dead animal
                self.animals.remove(index);

                // Return meat drop info
                if let Some((meat_type, min_qty, max_qty)) = meat_drop {
                    let qty = self.rng.gen_range(min_qty..=max_qty);
                    return Some((death_pos, meat_type, qty));
                }
            }
        }
        None
    }

    pub fn get_animals_mut(&mut self) -> &mut [Animal] {
        &mut self.animals
    }
}
