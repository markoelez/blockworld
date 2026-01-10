use noise::{NoiseFn, Perlin};
use cgmath::{Vector3, Point3};
use std::collections::HashMap;
use rand::Rng;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BlockType {
    Air,
    Grass,
    Dirt,
    Stone,
    Wood,
    Leaves,
    Barrier, // Invisible barrier block
    Water,
    Sand,
    Snow,
    Ice,
    Cobblestone,
    Coal,
    Iron,
    Gold,
    Diamond,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Biome {
    Plains,
    Forest,
    Desert,
    Mountains,
    Tundra,
    Ocean,
}

pub struct Chunk {
    pub blocks: Vec<Vec<Vec<BlockType>>>,
    pub position: Vector3<i32>,
    pub dirty: bool, // Needs mesh regeneration
    pub mesh_generated: bool,
}

pub struct World {
    pub chunks: HashMap<(i32, i32), Chunk>, // Use HashMap for O(1) chunk access
    noise: Perlin,
    tree_noise: Perlin,
    biome_noise: Perlin,
    temperature_noise: Perlin,
    humidity_noise: Perlin,
    cave_noise: Perlin,
    ore_noise: Perlin,
    render_distance: i32, // In chunks
    player_chunk_pos: (i32, i32),
    block_damage: HashMap<(i32, i32, i32), f32>,
}

impl World {
    pub const CHUNK_SIZE: usize = 16;
    pub const CHUNK_HEIGHT: usize = 64;
    pub const SEA_LEVEL: usize = 25;
    
    pub fn new() -> Self {
        let mut rng = rand::thread_rng();
        let mut world = Self {
            chunks: HashMap::new(),
            noise: Perlin::new(rng.gen()),
            tree_noise: Perlin::new(rng.gen()),
            biome_noise: Perlin::new(rng.gen()),
            temperature_noise: Perlin::new(rng.gen()),
            humidity_noise: Perlin::new(rng.gen()),
            cave_noise: Perlin::new(rng.gen()),
            ore_noise: Perlin::new(rng.gen()),
            render_distance: 6, // 6 chunk radius = 13x13 chunks visible
            player_chunk_pos: (0, 0),
            block_damage: HashMap::new(),
        };
        
        // Load initial chunks around spawn
        world.update_loaded_chunks(Point3::new(0.0, 0.0, 0.0));
        
        world
    }
    
    pub fn update_loaded_chunks(&mut self, player_pos: Point3<f32>) {
        let chunk_x = (player_pos.x as i32).div_euclid(Self::CHUNK_SIZE as i32);
        let chunk_z = (player_pos.z as i32).div_euclid(Self::CHUNK_SIZE as i32);
        let new_player_chunk = (chunk_x, chunk_z);
        
        // Only update if player moved to a different chunk
        if new_player_chunk != self.player_chunk_pos {
            self.load_chunks_around(chunk_x, chunk_z);
        }
    }
    
    /// Force load chunks around a position (used for initial spawn)
    pub fn force_load_chunks_at(&mut self, pos: Point3<f32>) {
        let chunk_x = (pos.x as i32).div_euclid(Self::CHUNK_SIZE as i32);
        let chunk_z = (pos.z as i32).div_euclid(Self::CHUNK_SIZE as i32);
        self.load_chunks_around(chunk_x, chunk_z);
    }
    
    fn load_chunks_around(&mut self, chunk_x: i32, chunk_z: i32) {
        self.player_chunk_pos = (chunk_x, chunk_z);
        
        // Load chunks in render distance
        for x in (chunk_x - self.render_distance)..=(chunk_x + self.render_distance) {
            for z in (chunk_z - self.render_distance)..=(chunk_z + self.render_distance) {
                let chunk_key = (x, z);
                if !self.chunks.contains_key(&chunk_key) {
                    self.load_chunk(x, z);
                }
            }
        }
        
        // Unload chunks outside render distance
        let chunks_to_unload: Vec<(i32, i32)> = self.chunks.keys()
            .filter(|(x, z)| {
                let dx = (*x - chunk_x).abs();
                let dz = (*z - chunk_z).abs();
                dx > self.render_distance + 1 || dz > self.render_distance + 1
            })
            .cloned()
            .collect();
        
        for chunk_key in chunks_to_unload {
            self.chunks.remove(&chunk_key);
        }
    }
    
    fn load_chunk(&mut self, chunk_x: i32, chunk_z: i32) {
        let mut chunk = self.generate_chunk_data(chunk_x, chunk_z);
        
        // Generate trees for this chunk
        self.generate_trees_for_chunk(&mut chunk);
        
        self.chunks.insert((chunk_x, chunk_z), chunk);
    }
    
    pub fn get_loaded_chunks(&self) -> impl Iterator<Item = &Chunk> {
        self.chunks.values()
    }
    
    pub fn _get_loaded_chunks_mut(&mut self) -> impl Iterator<Item = &mut Chunk> {
        self.chunks.values_mut()
    }
    
    pub fn _get_chunks(&self) -> &HashMap<(i32, i32), Chunk> {
        &self.chunks
    }
    
    pub fn _get_chunk(&self, chunk_x: i32, chunk_z: i32) -> Option<&Chunk> {
        self.chunks.get(&(chunk_x, chunk_z))
    }
    
    fn get_biome(&self, world_x: f64, world_z: f64) -> Biome {
        let temperature = self.temperature_noise.get([world_x * 0.005, world_z * 0.005]);
        let humidity = self.humidity_noise.get([world_x * 0.007, world_z * 0.007]);
        let biome_value = self.biome_noise.get([world_x * 0.003, world_z * 0.003]);
        
        // Determine biome based on temperature, humidity, and additional noise
        match (temperature, humidity, biome_value) {
            (t, _, _) if t < -0.4 => Biome::Tundra,
            (t, h, _b) if t > 0.3 && h < -0.2 => Biome::Desert,
            (_, h, b) if h > 0.3 && b > 0.2 => Biome::Forest,
            (t, _, b) if t > 0.0 && b > 0.4 => Biome::Mountains,
            (_, _, b) if b < -0.5 => Biome::Ocean,
            _ => Biome::Plains,
        }
    }
    
    fn get_height_for_biome(&self, world_x: f64, world_z: f64, biome: Biome) -> usize {
        let base_noise = self.noise.get([world_x * 0.01, world_z * 0.01]);
        let detail_noise = self.noise.get([world_x * 0.05, world_z * 0.05]) * 0.3;
        
        let base_height = match biome {
            Biome::Plains => base_noise * 8.0 + 28.0,
            Biome::Forest => base_noise * 12.0 + 30.0,
            Biome::Desert => base_noise * 6.0 + 26.0,
            Biome::Mountains => {
                let mountain_noise = self.noise.get([world_x * 0.003, world_z * 0.003]);
                base_noise * 25.0 + mountain_noise * 15.0 + 35.0
            },
            Biome::Tundra => base_noise * 10.0 + 32.0,
            Biome::Ocean => base_noise * 5.0 + 20.0,
        };
        
        ((base_height + detail_noise) as usize).min(Self::CHUNK_HEIGHT - 1).max(0)
    }
    
    fn generate_chunk_data(&self, chunk_x: i32, chunk_z: i32) -> Chunk {
        let mut blocks = vec![vec![vec![BlockType::Air; Self::CHUNK_SIZE]; Self::CHUNK_HEIGHT]; Self::CHUNK_SIZE];
        
        for x in 0..Self::CHUNK_SIZE {
            for z in 0..Self::CHUNK_SIZE {
                let world_x = (chunk_x * Self::CHUNK_SIZE as i32 + x as i32) as f64;
                let world_z = (chunk_z * Self::CHUNK_SIZE as i32 + z as i32) as f64;
                
                let biome = self.get_biome(world_x, world_z);
                let height = self.get_height_for_biome(world_x, world_z, biome);
                
                // Generate cave system
                let has_cave = self.generate_caves(world_x, world_z);
                
                for y in 0..Self::CHUNK_HEIGHT {
                    if y <= height {
                        // Check for caves
                        if has_cave.contains(&y) {
                            blocks[x][y][z] = BlockType::Air;
                            continue;
                        }
                        
                        // Generate ores in stone layer
                        if y < height - 5 {
                            if let Some(ore) = self.generate_ore(world_x, y as f64, world_z) {
                                blocks[x][y][z] = ore;
                                continue;
                            }
                        }
                        
                        // Generate terrain based on biome
                        blocks[x][y][z] = if y == height {
                            self.get_surface_block(biome, height, world_x, world_z)
                        } else if y > height - 3 {
                            self.get_subsurface_block(biome)
                        } else {
                            BlockType::Stone
                        };
                    } else if y <= Self::SEA_LEVEL && height < Self::SEA_LEVEL {
                        blocks[x][y][z] = match biome {
                            Biome::Tundra => BlockType::Ice,
                            _ => BlockType::Water,
                        };
                    } else {
                        blocks[x][y][z] = BlockType::Air;
                    }
                }
            }
        }
        
        Chunk {
            blocks,
            position: Vector3::new(chunk_x, 0, chunk_z),
            dirty: true,
            mesh_generated: false,
        }
    }
    
    fn is_near_water(&self, world_x: f64, world_z: f64) -> bool {
        // Check if this position is close to water level terrain
        let current_height = self.get_height_for_biome(world_x, world_z, self.get_biome(world_x, world_z));
        
        // Check surrounding positions for water
        let check_radius = 3.0;
        for dx in -2..=2 {
            for dz in -2..=2 {
                if dx == 0 && dz == 0 { continue; }
                
                let check_x = world_x + dx as f64;
                let check_z = world_z + dz as f64;
                let check_biome = self.get_biome(check_x, check_z);
                let check_height = self.get_height_for_biome(check_x, check_z, check_biome);
                
                // If nearby terrain is below sea level, this could be a water edge
                if check_height < Self::SEA_LEVEL && current_height >= Self::SEA_LEVEL {
                    let distance = ((dx * dx + dz * dz) as f64).sqrt();
                    if distance <= check_radius {
                        return true;
                    }
                }
            }
        }
        false
    }
    
    fn get_surface_block(&self, biome: Biome, height: usize, world_x: f64, world_z: f64) -> BlockType {
        // Check if this location should have sand due to water proximity
        if height >= Self::SEA_LEVEL && height <= Self::SEA_LEVEL + 2 {
            if self.is_near_water(world_x, world_z) {
                return BlockType::Sand;
            }
        }
        
        match biome {
            Biome::Plains | Biome::Forest => {
                if height < Self::SEA_LEVEL {
                    BlockType::Dirt
                } else {
                    BlockType::Grass
                }
            },
            Biome::Desert => BlockType::Sand,
            Biome::Mountains => {
                if height > 45 {
                    BlockType::Snow
                } else {
                    BlockType::Stone
                }
            },
            Biome::Tundra => BlockType::Snow,
            Biome::Ocean => BlockType::Sand,
        }
    }
    
    fn get_subsurface_block(&self, biome: Biome) -> BlockType {
        match biome {
            Biome::Desert | Biome::Ocean => BlockType::Sand,
            _ => BlockType::Dirt,
        }
    }
    
    fn generate_caves(&self, world_x: f64, world_z: f64) -> Vec<usize> {
        let mut cave_levels = Vec::new();
        
        // Generate cave systems at different depths
        for depth in [15, 25, 35] {
            let cave_noise = self.cave_noise.get([world_x * 0.02, depth as f64 * 0.1, world_z * 0.02]);
            let tunnel_noise = self.cave_noise.get([world_x * 0.05, depth as f64 * 0.05, world_z * 0.05]);
            
            if cave_noise > 0.4 || tunnel_noise > 0.6 {
                // Add cave at this depth and nearby levels
                for y_offset in -2..=2 {
                    let cave_y = (depth as i32 + y_offset) as usize;
                    if cave_y < Self::CHUNK_HEIGHT {
                        cave_levels.push(cave_y);
                    }
                }
            }
        }
        
        cave_levels
    }
    
    fn generate_ore(&self, world_x: f64, world_y: f64, world_z: f64) -> Option<BlockType> {
        let ore_noise = self.ore_noise.get([world_x * 0.1, world_y * 0.1, world_z * 0.1]);
        let depth_factor = (Self::CHUNK_HEIGHT as f64 - world_y) / Self::CHUNK_HEIGHT as f64;
        
        // Different ores at different depths
        match (ore_noise, depth_factor) {
            (n, d) if n > 0.85 && d > 0.8 => Some(BlockType::Diamond),
            (n, d) if n > 0.75 && d > 0.6 => Some(BlockType::Gold),
            (n, d) if n > 0.65 && d > 0.4 => Some(BlockType::Iron),
            (n, d) if n > 0.55 && d > 0.2 => Some(BlockType::Coal),
            _ => None,
        }
    }
    
    fn generate_trees_for_chunk(&self, chunk: &mut Chunk) {
        let chunk_world_x = chunk.position.x * Self::CHUNK_SIZE as i32;
        let chunk_world_z = chunk.position.z * Self::CHUNK_SIZE as i32;
        
        for x in 0..Self::CHUNK_SIZE {
            for z in 0..Self::CHUNK_SIZE {
                let world_x = chunk_world_x + x as i32;
                let world_z = chunk_world_z + z as i32;
                
                let biome = self.get_biome(world_x as f64, world_z as f64);
                
                // Different tree densities and types per biome
                let (tree_threshold, structure_threshold) = match biome {
                    Biome::Forest => (0.4, 0.85), // Dense trees, occasional clearings
                    Biome::Plains => (0.75, 0.9), // Sparse trees, villages
                    Biome::Desert => (0.95, 0.8), // Very rare cacti, ruins
                    Biome::Mountains => (0.8, 0.9), // Pine trees, mountain structures
                    Biome::Tundra => (0.85, 0.9), // Sparse pine, igloos
                    Biome::Ocean => (1.0, 1.0), // No trees
                };
                
                let tree_density = self.tree_noise.get([world_x as f64 * 0.05, world_z as f64 * 0.05]);
                let structure_density = self.tree_noise.get([world_x as f64 * 0.02, world_z as f64 * 0.02]);
                
                // Generate special structures
                if structure_density > structure_threshold {
                    if self.is_suitable_for_structure_in_chunk(chunk, x, z, biome) {
                        self.place_structure_in_chunk(chunk, x, z, world_x, world_z, biome);
                        continue; // Don't place trees where structures are
                    }
                }
                
                // Generate trees
                if tree_density > tree_threshold && self.is_suitable_for_tree_in_chunk(chunk, x, z, biome) {
                    self.place_tree_in_chunk(chunk, x, z, world_x, world_z, biome);
                }
            }
        }
    }
    
    fn is_suitable_for_tree_in_chunk(&self, chunk: &Chunk, local_x: usize, local_z: usize, biome: Biome) -> bool {
        // Find the surface block
        for y in (0..Self::CHUNK_HEIGHT).rev() {
            let block = chunk.blocks[local_x][y][local_z];
            
            let suitable_surface = match biome {
                Biome::Forest | Biome::Plains => block == BlockType::Grass,
                Biome::Desert => block == BlockType::Sand,
                Biome::Mountains | Biome::Tundra => block == BlockType::Grass || block == BlockType::Snow,
                Biome::Ocean => false,
            };
            
            if suitable_surface {
                // Check if there's enough space above for a tree
                let tree_height = match biome {
                    Biome::Desert => 3, // Cacti are shorter
                    Biome::Mountains | Biome::Tundra => 8, // Pine trees are taller
                    _ => 6,
                };
                
                for check_y in (y + 1)..=(y + tree_height).min(Self::CHUNK_HEIGHT - 1) {
                    if chunk.blocks[local_x][check_y][local_z] != BlockType::Air {
                        return false;
                    }
                }
                return true;
            } else if block != BlockType::Air {
                return false; // Hit non-suitable solid block
            }
        }
        false
    }
    
    fn is_suitable_for_structure_in_chunk(&self, chunk: &Chunk, local_x: usize, local_z: usize, biome: Biome) -> bool {
        // Need flat area for structures
        for dx in -2..=2 {
            for dz in -2..=2 {
                let check_x = local_x as i32 + dx;
                let check_z = local_z as i32 + dz;
                
                if check_x < 0 || check_x >= Self::CHUNK_SIZE as i32 || 
                   check_z < 0 || check_z >= Self::CHUNK_SIZE as i32 {
                    continue;
                }
                
                // Find surface height at this position
                let mut found_surface = false;
                for y in (0..Self::CHUNK_HEIGHT).rev() {
                    let block = chunk.blocks[check_x as usize][y][check_z as usize];
                    if block != BlockType::Air {
                        let suitable = match biome {
                            Biome::Plains => block == BlockType::Grass,
                            Biome::Desert => block == BlockType::Sand,
                            Biome::Tundra => block == BlockType::Snow,
                            _ => block == BlockType::Grass || block == BlockType::Stone,
                        };
                        if suitable {
                            found_surface = true;
                        }
                        break;
                    }
                }
                
                if !found_surface {
                    return false;
                }
            }
        }
        true
    }
    
    fn place_tree_in_chunk(&self, chunk: &mut Chunk, local_x: usize, local_z: usize, world_x: i32, world_z: i32, biome: Biome) {
        // Find the surface height
        let mut surface_y = None;
        for y in (0..Self::CHUNK_HEIGHT).rev() {
            let block = chunk.blocks[local_x][y][local_z];
            let suitable = match biome {
                Biome::Forest | Biome::Plains => block == BlockType::Grass,
                Biome::Desert => block == BlockType::Sand,
                Biome::Mountains | Biome::Tundra => block == BlockType::Grass || block == BlockType::Snow,
                _ => false,
            };
            
            if suitable {
                surface_y = Some(y);
                break;
            } else if block != BlockType::Air {
                return; // Not suitable
            }
        }
        
        if let Some(ground_y) = surface_y {
            match biome {
                Biome::Desert => self.place_cactus(chunk, local_x, local_z, ground_y, world_x, world_z),
                Biome::Mountains | Biome::Tundra => self.place_pine_tree(chunk, local_x, local_z, ground_y, world_x, world_z),
                _ => self.place_oak_tree(chunk, local_x, local_z, ground_y, world_x, world_z),
            }
        }
    }
    
    fn place_oak_tree(&self, chunk: &mut Chunk, local_x: usize, local_z: usize, ground_y: usize, world_x: i32, world_z: i32) {
        let trunk_height = 4 + (self.tree_noise.get([world_x as f64 * 0.1, world_z as f64 * 0.1]) * 2.0) as usize;
        let trunk_height = trunk_height.max(3).min(6);
        
        // Place trunk
        for y in 1..=trunk_height {
            if ground_y + y < Self::CHUNK_HEIGHT {
                chunk.blocks[local_x][ground_y + y][local_z] = BlockType::Wood;
            }
        }
        
        // Place leaves in a sphere-like pattern around the top
        let leaves_center_y = ground_y + trunk_height;
        let leaves_radius = 2;
        
        for dx in -(leaves_radius as i32)..=(leaves_radius as i32) {
            for dz in -(leaves_radius as i32)..=(leaves_radius as i32) {
                for dy in -(leaves_radius as i32)..=(leaves_radius as i32) {
                    let leaf_x = local_x as i32 + dx;
                    let leaf_z = local_z as i32 + dz;
                    let leaf_y = leaves_center_y as i32 + dy;
                    
                    // Check bounds
                    if leaf_x >= 0 && leaf_x < Self::CHUNK_SIZE as i32 && 
                       leaf_z >= 0 && leaf_z < Self::CHUNK_SIZE as i32 &&
                       leaf_y >= 0 && leaf_y < Self::CHUNK_HEIGHT as i32 {
                        
                        // Create a roughly spherical shape
                        let distance_sq = dx * dx + dy * dy + dz * dz;
                        if distance_sq <= (leaves_radius * leaves_radius) as i32 {
                            // Don't replace trunk blocks
                            if dx == 0 && dz == 0 && dy <= 0 {
                                continue;
                            }
                            
                            // Add some randomness to leaf placement
                            let leaf_noise = self.tree_noise.get([
                                (world_x + dx) as f64 * 0.3, 
                                leaf_y as f64 * 0.3, 
                                (world_z + dz) as f64 * 0.3
                            ]);
                            if leaf_noise > -0.3 {
                                if chunk.blocks[leaf_x as usize][leaf_y as usize][leaf_z as usize] == BlockType::Air {
                                    chunk.blocks[leaf_x as usize][leaf_y as usize][leaf_z as usize] = BlockType::Leaves;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    
    fn place_pine_tree(&self, chunk: &mut Chunk, local_x: usize, local_z: usize, ground_y: usize, world_x: i32, world_z: i32) {
        let trunk_height = 6 + (self.tree_noise.get([world_x as f64 * 0.1, world_z as f64 * 0.1]) * 3.0) as usize;
        let trunk_height = trunk_height.max(5).min(8);
        
        // Place trunk
        for y in 1..=trunk_height {
            if ground_y + y < Self::CHUNK_HEIGHT {
                chunk.blocks[local_x][ground_y + y][local_z] = BlockType::Wood;
            }
        }
        
        // Place leaves in a conical pattern (pine tree shape)
        for layer in 0..=(trunk_height / 2) {
            let layer_y = ground_y + trunk_height - layer;
            let layer_radius = (layer / 2 + 1).min(3);
            
            for dx in -(layer_radius as i32)..=(layer_radius as i32) {
                for dz in -(layer_radius as i32)..=(layer_radius as i32) {
                    let leaf_x = local_x as i32 + dx;
                    let leaf_z = local_z as i32 + dz;
                    
                    if leaf_x >= 0 && leaf_x < Self::CHUNK_SIZE as i32 && 
                       leaf_z >= 0 && leaf_z < Self::CHUNK_SIZE as i32 &&
                       layer_y < Self::CHUNK_HEIGHT {
                        
                        let distance_sq = dx * dx + dz * dz;
                        if distance_sq <= (layer_radius * layer_radius) as i32 {
                            // Don't replace trunk blocks
                            if dx == 0 && dz == 0 {
                                continue;
                            }
                            
                            if chunk.blocks[leaf_x as usize][layer_y][leaf_z as usize] == BlockType::Air {
                                chunk.blocks[leaf_x as usize][layer_y][leaf_z as usize] = BlockType::Leaves;
                            }
                        }
                    }
                }
            }
        }
    }
    
    fn place_cactus(&self, chunk: &mut Chunk, local_x: usize, local_z: usize, ground_y: usize, world_x: i32, world_z: i32) {
        let cactus_height = 2 + (self.tree_noise.get([world_x as f64 * 0.15, world_z as f64 * 0.15]) * 2.0) as usize;
        let cactus_height = cactus_height.max(1).min(4);
        
        // Place cactus trunk (using wood blocks for now - could add cactus block type later)
        for y in 1..=cactus_height {
            if ground_y + y < Self::CHUNK_HEIGHT {
                chunk.blocks[local_x][ground_y + y][local_z] = BlockType::Leaves; // Using leaves as cactus
            }
        }
    }
    
    fn place_structure_in_chunk(&self, chunk: &mut Chunk, local_x: usize, local_z: usize, _world_x: i32, _world_z: i32, biome: Biome) {
        // Find the surface height
        let mut surface_y = None;
        for y in (0..Self::CHUNK_HEIGHT).rev() {
            let block = chunk.blocks[local_x][y][local_z];
            if block != BlockType::Air {
                surface_y = Some(y);
                break;
            }
        }
        
        if let Some(ground_y) = surface_y {
            match biome {
                Biome::Plains => self.place_village_structure(chunk, local_x, local_z, ground_y),
                Biome::Desert => self.place_desert_ruin(chunk, local_x, local_z, ground_y),
                Biome::Mountains => self.place_mountain_shrine(chunk, local_x, local_z, ground_y),
                Biome::Tundra => self.place_igloo(chunk, local_x, local_z, ground_y),
                _ => {} // No structures for other biomes
            }
        }
    }
    
    fn place_village_structure(&self, chunk: &mut Chunk, local_x: usize, local_z: usize, ground_y: usize) {
        // Simple house structure
        for dx in -1..=1 {
            for dz in -1..=1 {
                for dy in 1..=3 {
                    let x = local_x as i32 + dx;
                    let z = local_z as i32 + dz;
                    let y = ground_y + dy;
                    
                    if x >= 0 && x < Self::CHUNK_SIZE as i32 && 
                       z >= 0 && z < Self::CHUNK_SIZE as i32 && 
                       y < Self::CHUNK_HEIGHT {
                        
                        // Walls
                        if dx.abs() == 1 || dz.abs() == 1 {
                            if dy <= 2 {
                                chunk.blocks[x as usize][y][z as usize] = BlockType::Wood;
                            } else {
                                chunk.blocks[x as usize][y][z as usize] = BlockType::Leaves; // Roof
                            }
                        }
                    }
                }
            }
        }
    }
    
    fn place_desert_ruin(&self, chunk: &mut Chunk, local_x: usize, local_z: usize, ground_y: usize) {
        // Broken stone structure
        for dx in -1..=1 {
            for dz in -1..=1 {
                let x = local_x as i32 + dx;
                let z = local_z as i32 + dz;
                let y = ground_y + 1;
                
                if x >= 0 && x < Self::CHUNK_SIZE as i32 && 
                   z >= 0 && z < Self::CHUNK_SIZE as i32 && 
                   y < Self::CHUNK_HEIGHT {
                    
                    // Partial walls
                    if (dx.abs() == 1 || dz.abs() == 1) && (dx + dz) % 2 == 0 {
                        chunk.blocks[x as usize][y][z as usize] = BlockType::Cobblestone;
                    }
                }
            }
        }
    }
    
    fn place_mountain_shrine(&self, chunk: &mut Chunk, local_x: usize, local_z: usize, ground_y: usize) {
        // Stone pillar
        for dy in 1..=4 {
            let y = ground_y + dy;
            if y < Self::CHUNK_HEIGHT {
                chunk.blocks[local_x][y][local_z] = BlockType::Stone;
            }
        }
    }
    
    fn place_igloo(&self, chunk: &mut Chunk, local_x: usize, local_z: usize, ground_y: usize) {
        // Snow dome
        for dx in -1..=1 {
            for dz in -1..=1 {
                for dy in 1..=2 {
                    let x = local_x as i32 + dx;
                    let z = local_z as i32 + dz;
                    let y = ground_y + dy;
                    
                    if x >= 0 && x < Self::CHUNK_SIZE as i32 && 
                       z >= 0 && z < Self::CHUNK_SIZE as i32 && 
                       y < Self::CHUNK_HEIGHT {
                        
                        // Dome shape
                        if dx.abs() + dz.abs() + (dy as i32) <= 2 {
                            chunk.blocks[x as usize][y][z as usize] = BlockType::Snow;
                        }
                    }
                }
            }
        }
    }
    
    pub fn set_block(&mut self, x: i32, y: i32, z: i32, block_type: BlockType) {
        let chunk_x = x.div_euclid(Self::CHUNK_SIZE as i32);
        let chunk_z = z.div_euclid(Self::CHUNK_SIZE as i32);
        let local_x = x.rem_euclid(Self::CHUNK_SIZE as i32) as usize;
        let local_z = z.rem_euclid(Self::CHUNK_SIZE as i32) as usize;
        
        if y < 0 || y >= Self::CHUNK_HEIGHT as i32 {
            return;
        }
        
        if let Some(chunk) = self.chunks.get_mut(&(chunk_x, chunk_z)) {
            chunk.blocks[local_x][y as usize][local_z] = block_type;
            chunk.dirty = true; // Mark chunk as needing mesh regeneration
        }
    }
    
    pub fn get_block(&self, x: i32, y: i32, z: i32) -> Option<BlockType> {
        let chunk_x = x.div_euclid(Self::CHUNK_SIZE as i32);
        let chunk_z = z.div_euclid(Self::CHUNK_SIZE as i32);
        let local_x = x.rem_euclid(Self::CHUNK_SIZE as i32) as usize;
        let local_z = z.rem_euclid(Self::CHUNK_SIZE as i32) as usize;
        
        if y < 0 || y >= Self::CHUNK_HEIGHT as i32 {
            return None;
        }
        
        self.chunks.get(&(chunk_x, chunk_z))
            .and_then(|chunk| chunk.blocks.get(local_x))
            .and_then(|yz| yz.get(y as usize))
            .and_then(|z_blocks| z_blocks.get(local_z))
            .copied()
    }
    
    pub fn _has_terrain_at(&self, x: i32, z: i32) -> bool {
        // Check if there's any solid terrain at this x,z position
        for y in 0..Self::CHUNK_HEIGHT {
            if let Some(block) = self.get_block(x, y as i32, z) {
                if block != BlockType::Air && block != BlockType::Barrier && block != BlockType::Wood && block != BlockType::Leaves {
                    return true;
                }
            }
        }
        false
    }
    
    pub fn _get_terrain_bounds(&self) -> (i32, i32, i32, i32) {
        // Return min_x, max_x, min_z, max_z of actual walkable terrain
        let mut min_x = i32::MAX;
        let mut max_x = i32::MIN;
        let mut min_z = i32::MAX;
        let mut max_z = i32::MIN;
        
        for chunk in self.chunks.values() {
            let chunk_world_x = chunk.position.x * Self::CHUNK_SIZE as i32;
            let chunk_world_z = chunk.position.z * Self::CHUNK_SIZE as i32;
            
            for x in 0..Self::CHUNK_SIZE {
                for z in 0..Self::CHUNK_SIZE {
                    let world_x = chunk_world_x + x as i32;
                    let world_z = chunk_world_z + z as i32;
                    
                    // Check for surface blocks (has solid ground with air above)
                    if self._has_walkable_surface_at(world_x, world_z) {
                        min_x = min_x.min(world_x);
                        max_x = max_x.max(world_x);
                        min_z = min_z.min(world_z);
                        max_z = max_z.max(world_z);
                    }
                }
            }
        }
        
        // Add safety margin to ensure we stay on solid ground
        (min_x + 1, max_x - 1, min_z + 1, max_z - 1)
    }
    
    pub fn _has_walkable_surface_at(&self, x: i32, z: i32) -> bool {
        // Find the highest solid block and check if it has air above for walking
        for y in (0..Self::CHUNK_HEIGHT).rev() {
            if let Some(block) = self.get_block(x, y as i32, z) {
                if block != BlockType::Air && block != BlockType::Barrier && block != BlockType::Wood && block != BlockType::Leaves {
                    // Check if there's space above to walk
                    if let Some(above_block) = self.get_block(x, y as i32 + 1, z) {
                        if above_block == BlockType::Air {
                            return true;
                        }
                    } else {
                        // If no block above (at world edge), assume walkable
                        return true;
                    }
                    return false; // Found solid block but no air above
                }
            }
        }
        false
    }
    
    pub fn can_place_block_at(&self, x: i32, y: i32, z: i32) -> bool {
        if y < 0 || y >= Self::CHUNK_HEIGHT as i32 {
            return false;
        }
        
        if let Some(block) = self.get_block(x, y, z) {
            if block != BlockType::Air {
                return false;
            }
        } else {
            return false;
        }
        
        // Prevent placing directly on water surface
        if let Some(below) = self.get_block(x, y - 1, z) {
            if below == BlockType::Water {
                return false;
            }
        }
        
        true
    }
    
    pub fn can_destroy_block_at(&self, x: i32, y: i32, z: i32) -> bool {
        // Check if we can destroy a block at this position
        if y < 0 || y >= Self::CHUNK_HEIGHT as i32 {
            return false;
        }
        
        // Can destroy any block except air and barriers
        if let Some(block) = self.get_block(x, y, z) {
            block != BlockType::Air && block != BlockType::Barrier && block != BlockType::Water
        } else {
            false // Outside loaded chunks
        }
    }
    
    pub fn place_block(&mut self, x: i32, y: i32, z: i32, block_type: BlockType) -> bool {
        if self.can_place_block_at(x, y, z) {
            self.set_block(x, y, z, block_type);
            true
        } else {
            false
        }
    }
    
    pub fn get_hardness(block_type: BlockType) -> f32 {
        match block_type {
            BlockType::Leaves => 1.0,
            BlockType::Wood => 5.0,
            BlockType::Dirt => 3.0,
            BlockType::Grass => 3.0,
            BlockType::Stone => 10.0,
            _ => 1.0,
        }
    }

    pub fn damage_block(&mut self, x: i32, y: i32, z: i32) -> Option<BlockType> {
        if !self.can_destroy_block_at(x, y, z) {
            return None;
        }

        let block_type = self.get_block(x, y, z)?;
        let pos = (x, y, z);
        let current_damage = self.block_damage.get(&pos).copied().unwrap_or(0.0);
        let new_damage = current_damage + 1.0;
        let hardness = Self::get_hardness(block_type);

        if new_damage >= hardness {
            self.set_block(x, y, z, BlockType::Air);
            self.block_damage.remove(&pos);
            Some(block_type)
        } else {
            self.block_damage.insert(pos, new_damage);
            let chunk_x = x.div_euclid(Self::CHUNK_SIZE as i32);
            let chunk_z = z.div_euclid(Self::CHUNK_SIZE as i32);
            if let Some(chunk) = self.chunks.get_mut(&(chunk_x, chunk_z)) {
                chunk.dirty = true;
            }
            None
        }
    }

    pub fn get_block_damage(&self, x: i32, y: i32, z: i32) -> f32 {
        self.block_damage.get(&(x, y, z)).copied().unwrap_or(0.0)
    }

    /// Find a valid spawn position on solid ground with clearance above
    pub fn find_spawn_position(&self) -> Point3<f32> {
        // Search in a spiral pattern from origin to find a good spawn spot
        for radius in 0i32..30 {
            for dx in -radius..=radius {
                for dz in -radius..=radius {
                    // Only check the edge of each radius (spiral pattern)
                    if radius > 0 && dx.abs() < radius && dz.abs() < radius {
                        continue;
                    }
                    
                    let x = dx;
                    let z = dz;
                    
                    // Find the highest solid, walkable block (search from top down)
                    for y in (1..Self::CHUNK_HEIGHT as i32 - 4).rev() {
                        if let Some(block) = self.get_block(x, y, z) {
                            // Must be solid ground (not air, water, trees, etc.)
                            let is_solid_ground = matches!(block, 
                                BlockType::Grass | BlockType::Dirt | BlockType::Stone | 
                                BlockType::Sand | BlockType::Snow | BlockType::Cobblestone
                            );
                            
                            if is_solid_ground {
                                // Check for 3 blocks of air above for player clearance
                                let mut has_clearance = true;
                                for check_y in 1..=3 {
                                    if let Some(above) = self.get_block(x, y + check_y, z) {
                                        if above != BlockType::Air {
                                            has_clearance = false;
                                            break;
                                        }
                                    } else {
                                        // Block not loaded, skip this position
                                        has_clearance = false;
                                        break;
                                    }
                                }
                                
                                if has_clearance {
                                    // Found valid spawn - position player standing on the block
                                    // Ground block top is at y+1, player feet at y+1, eyes at y+1+1.8
                                    return Point3::new(
                                        x as f32 + 0.5,
                                        y as f32 + 1.0 + 1.8, // Stand on top of block
                                        z as f32 + 0.5
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        
        // Fallback: spawn at a safe default position
        Point3::new(0.5, 35.0, 0.5)
    }
}