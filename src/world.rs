use noise::{NoiseFn, Perlin};
use cgmath::{Vector3, Point3};
use std::collections::HashMap;

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
    render_distance: i32, // In chunks
    player_chunk_pos: (i32, i32),
}

impl World {
    pub const CHUNK_SIZE: usize = 16;
    pub const CHUNK_HEIGHT: usize = 64;
    pub const SEA_LEVEL: usize = 25;
    
    pub fn new() -> Self {
        let mut world = Self {
            chunks: HashMap::new(),
            noise: Perlin::new(0),
            tree_noise: Perlin::new(42),
            render_distance: 6, // 6 chunk radius = 13x13 chunks visible
            player_chunk_pos: (0, 0),
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
            self.player_chunk_pos = new_player_chunk;
            
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
    
    pub fn get_loaded_chunks_mut(&mut self) -> impl Iterator<Item = &mut Chunk> {
        self.chunks.values_mut()
    }
    
    pub fn get_chunks(&self) -> &HashMap<(i32, i32), Chunk> {
        &self.chunks
    }
    
    pub fn get_chunk(&self, chunk_x: i32, chunk_z: i32) -> Option<&Chunk> {
        self.chunks.get(&(chunk_x, chunk_z))
    }
    
    fn generate_chunk_data(&self, chunk_x: i32, chunk_z: i32) -> Chunk {
        let mut blocks = vec![vec![vec![BlockType::Air; Self::CHUNK_SIZE]; Self::CHUNK_HEIGHT]; Self::CHUNK_SIZE];
        
        for x in 0..Self::CHUNK_SIZE {
            for z in 0..Self::CHUNK_SIZE {
                let world_x = (chunk_x * Self::CHUNK_SIZE as i32 + x as i32) as f64;
                let world_z = (chunk_z * Self::CHUNK_SIZE as i32 + z as i32) as f64;
                
                // Generate height using Perlin noise
                let height = ((self.noise.get([world_x * 0.01, world_z * 0.01]) * 20.0 + 25.0) as usize).min(Self::CHUNK_HEIGHT - 1);
                
                for y in 0..Self::CHUNK_HEIGHT {
                    if y <= height {
                        blocks[x][y][z] = if y == height {
                            if height < Self::SEA_LEVEL {
                                BlockType::Dirt
                            } else {
                                BlockType::Grass
                            }
                        } else if y > height - 3 {
                            BlockType::Dirt
                        } else {
                            BlockType::Stone
                        };
                    } else if y <= Self::SEA_LEVEL && height < Self::SEA_LEVEL {
                        blocks[x][y][z] = BlockType::Water;
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
    
    fn generate_trees_for_chunk(&self, chunk: &mut Chunk) {
        let chunk_world_x = chunk.position.x * Self::CHUNK_SIZE as i32;
        let chunk_world_z = chunk.position.z * Self::CHUNK_SIZE as i32;
        
        for x in 0..Self::CHUNK_SIZE {
            for z in 0..Self::CHUNK_SIZE {
                let world_x = chunk_world_x + x as i32;
                let world_z = chunk_world_z + z as i32;
                
                // Use tree noise to determine if a tree should spawn here
                let tree_density = self.tree_noise.get([world_x as f64 * 0.05, world_z as f64 * 0.05]);
                
                // Only place trees on grass blocks with proper spacing
                if tree_density > 0.6 && self.is_suitable_for_tree_in_chunk(chunk, x, z) {
                    self.place_tree_in_chunk(chunk, x, z, world_x, world_z);
                }
            }
        }
    }
    
    fn is_suitable_for_tree_in_chunk(&self, chunk: &Chunk, local_x: usize, local_z: usize) -> bool {
        // Find the surface block
        for y in (0..Self::CHUNK_HEIGHT).rev() {
            let block = chunk.blocks[local_x][y][local_z];
            if block == BlockType::Grass {
                // Check if there's enough space above for a tree
                let tree_height = 6;
                for check_y in (y + 1)..=(y + tree_height).min(Self::CHUNK_HEIGHT - 1) {
                    if chunk.blocks[local_x][check_y][local_z] != BlockType::Air {
                        return false;
                    }
                }
                return true;
            } else if block != BlockType::Air {
                return false; // Hit non-grass solid block
            }
        }
        false
    }
    
    fn place_tree_in_chunk(&self, chunk: &mut Chunk, local_x: usize, local_z: usize, world_x: i32, world_z: i32) {
        // Find the surface height
        let mut surface_y = None;
        for y in (0..Self::CHUNK_HEIGHT).rev() {
            if chunk.blocks[local_x][y][local_z] == BlockType::Grass {
                surface_y = Some(y);
                break;
            } else if chunk.blocks[local_x][y][local_z] != BlockType::Air {
                return; // Not suitable
            }
        }
        
        if let Some(ground_y) = surface_y {
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
    
    pub fn has_terrain_at(&self, x: i32, z: i32) -> bool {
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
    
    pub fn get_terrain_bounds(&self) -> (i32, i32, i32, i32) {
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
                    if self.has_walkable_surface_at(world_x, world_z) {
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
    
    pub fn has_walkable_surface_at(&self, x: i32, z: i32) -> bool {
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
    
    pub fn destroy_block(&mut self, x: i32, y: i32, z: i32) -> bool {
        if self.can_destroy_block_at(x, y, z) {
            self.set_block(x, y, z, BlockType::Air);
            true
        } else {
            false
        }
    }
}