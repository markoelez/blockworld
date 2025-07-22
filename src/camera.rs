use cgmath::{Matrix4, Vector3, Point3, Deg, perspective, InnerSpace};
use winit::event::VirtualKeyCode;
use wgpu::SurfaceConfiguration;
use crate::world::{World, BlockType};

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
    
    moving_forward: bool,
    moving_backward: bool,
    moving_left: bool,
    moving_right: bool,
    jump_pressed: bool,
    bob_time: f32,
}

impl Camera {
    pub fn new(config: &SurfaceConfiguration) -> Self {
        // Spawn player in air at origin - gravity will drop them to ground
        let spawn_x = 0.5;
        let spawn_z = 0.5;
        let spawn_y = 40.0; // High enough to be above most terrain
        
        println!("Spawning at ({}, {}, {}) - player will fall to ground", spawn_x, spawn_y, spawn_z);
        
        let mut camera = Self {
            position: Point3::new(spawn_x, spawn_y, spawn_z),
            yaw: 0.0,
            pitch: -20.0,
            aspect: config.width as f32 / config.height as f32,
            fovy: 70.0,
            znear: 0.1,
            zfar: 1000.0,
            view_proj: Matrix4::from_scale(1.0),
            
            velocity: Vector3::new(0.0, 0.0, 0.0),
            on_ground: false,
            
            moving_forward: false,
            moving_backward: false,
            moving_left: false,
            moving_right: false,
            jump_pressed: false,
            bob_time: 0.0,
        };
        camera.update_view_proj();
        camera
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
        self.pitch = self.pitch.max(-89.0).min(89.0);
        
        self.update_view_proj();
    }
    
    pub fn update(&mut self, dt: f32, world: &World) {
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
        
        let move_speed = if is_in_water { 2.0 } else { 4.3 };
        let mut horizontal_velocity = Vector3::new(0.0, 0.0, 0.0);
        if move_dir.magnitude() > 0.0 {
            horizontal_velocity = move_dir.normalize() * move_speed;
        }
        
        // Apply gravity
        const GRAVITY: f32 = 32.0; // Blocks per second squared
        const JUMP_VELOCITY: f32 = 10.0; // Initial jump velocity
        
        self.velocity.y -= GRAVITY * dt;
        
        if is_in_water {
            self.velocity.y += GRAVITY * 0.9 * dt;
            self.velocity.y *= 0.95;
            self.velocity.y += (self.bob_time * 3.0).sin() * 0.05;
        }
        
        // Jump if on ground or in water
        if self.jump_pressed && (self.on_ground || is_in_water) {
            self.velocity.y = if is_in_water { 5.0 } else { JUMP_VELOCITY };
        }
        
        // Collision detection constants
        const PLAYER_HEIGHT: f32 = 1.8;
        const PLAYER_WIDTH: f32 = 0.6;
        
        // Apply movement with collision detection
        let mut new_position = self.position;
        
        // Check X movement collision
        let test_x = self.position.x + horizontal_velocity.x * dt;
        let mut can_move_x = true;
        
        for dy in 0..2 { // Check collision for player height
            let check_y = (self.position.y - PLAYER_HEIGHT + dy as f32).floor() as i32;
            for dz in [-1, 0, 1] { // Check around player width
                let check_z = (self.position.z + dz as f32 * PLAYER_WIDTH * 0.5).floor() as i32;
                let check_x = if horizontal_velocity.x > 0.0 {
                    (test_x + PLAYER_WIDTH * 0.5).floor() as i32
                } else {
                    (test_x - PLAYER_WIDTH * 0.5).floor() as i32
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
            new_position.x = test_x;
        }
        
        // Check Z movement collision
        let test_z = self.position.z + horizontal_velocity.z * dt;
        let mut can_move_z = true;
        
        for dy in 0..2 { // Check collision for player height
            let check_y = (self.position.y - PLAYER_HEIGHT + dy as f32).floor() as i32;
            for dx in [-1, 0, 1] { // Check around player width
                let check_x = (new_position.x + dx as f32 * PLAYER_WIDTH * 0.5).floor() as i32;
                let check_z = if horizontal_velocity.z > 0.0 {
                    (test_z + PLAYER_WIDTH * 0.5).floor() as i32
                } else {
                    (test_z - PLAYER_WIDTH * 0.5).floor() as i32
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
            new_position.z = test_z;
        }
        
        // Apply Y movement and check vertical collisions
        new_position.y += self.velocity.y * dt;
        
        // Check ground collision
        self.on_ground = false;
        let feet_y = (new_position.y - PLAYER_HEIGHT).floor() as i32;
        
        for dx in [-1, 0, 1] {
            for dz in [-1, 0, 1] {
                let check_x = (new_position.x + dx as f32 * PLAYER_WIDTH * 0.3).floor() as i32;
                let check_z = (new_position.z + dz as f32 * PLAYER_WIDTH * 0.3).floor() as i32;
                
                if let Some(block) = world.get_block(check_x, feet_y, check_z) {
                    if block != BlockType::Air && block != BlockType::Water {
                        if self.velocity.y <= 0.0 {
                            new_position.y = feet_y as f32 + PLAYER_HEIGHT + 1.0;
                            self.velocity.y = 0.0;
                            self.on_ground = true;
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
                    if block != BlockType::Air && block != BlockType::Water && self.velocity.y > 0.0 {
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
        
        self.position = new_position;
        self.update_view_proj();
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
        let direction = self.get_look_direction();
        let step_size = 0.05; // Smaller steps for more precision
        let steps = (max_distance / step_size) as i32;
        
        let mut last_air_pos = None;
        
        for i in 0..steps {
            let distance = i as f32 * step_size;
            let check_pos = self.position + direction * distance;
            
            let block_x = check_pos.x.floor() as i32;
            let block_y = check_pos.y.floor() as i32;
            let block_z = check_pos.z.floor() as i32;
            
            if let Some(block) = world.get_block(block_x, block_y, block_z) {
                if block != BlockType::Air && block != BlockType::Barrier {
                    // Hit a solid block, return the last air position
                    return last_air_pos;
                } else if block == BlockType::Air {
                    // Update last known air position
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