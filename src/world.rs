use noise::{NoiseFn, Perlin, Simplex};
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
    Gravel,
    Clay,
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
    // Terrain noise layers
    continent_noise: Perlin,      // Large scale landmass shapes
    mountain_noise: Perlin,       // Mountain ranges
    hill_noise: Perlin,           // Rolling hills
    detail_noise: Perlin,         // Fine detail
    erosion_noise: Perlin,        // Erosion patterns
    ridge_noise: Simplex,         // Ridge/valley patterns
    // Feature noise
    river_noise: Perlin,          // River paths
    lake_noise: Perlin,           // Lake locations
    tree_noise: Perlin,
    biome_noise: Perlin,
    temperature_noise: Perlin,
    humidity_noise: Perlin,
    cave_noise: Perlin,
    ore_noise: Perlin,
    render_distance: i32,
    player_chunk_pos: (i32, i32),
    block_damage: HashMap<(i32, i32, i32), f32>,
    seed: u32,
}

impl World {
    pub const CHUNK_SIZE: usize = 16;
    pub const CHUNK_HEIGHT: usize = 128;  // Taller world for mountains
    pub const SEA_LEVEL: usize = 45;      // Higher sea level
    pub const BEACH_HEIGHT: usize = 48;   // Sand appears up to here near water

    pub fn new() -> Self {
        let mut rng = rand::thread_rng();
        let seed: u32 = rng.gen();

        let mut world = Self {
            chunks: HashMap::new(),
            // Terrain layers with different seeds for variety
            continent_noise: Perlin::new(seed),
            mountain_noise: Perlin::new(seed.wrapping_add(1)),
            hill_noise: Perlin::new(seed.wrapping_add(2)),
            detail_noise: Perlin::new(seed.wrapping_add(3)),
            erosion_noise: Perlin::new(seed.wrapping_add(4)),
            ridge_noise: Simplex::new(seed.wrapping_add(5)),
            river_noise: Perlin::new(seed.wrapping_add(6)),
            lake_noise: Perlin::new(seed.wrapping_add(7)),
            tree_noise: Perlin::new(seed.wrapping_add(8)),
            biome_noise: Perlin::new(seed.wrapping_add(9)),
            temperature_noise: Perlin::new(seed.wrapping_add(10)),
            humidity_noise: Perlin::new(seed.wrapping_add(11)),
            cave_noise: Perlin::new(seed.wrapping_add(12)),
            ore_noise: Perlin::new(seed.wrapping_add(13)),
            render_distance: 6,
            player_chunk_pos: (0, 0),
            block_damage: HashMap::new(),
            seed,
        };

        // Load initial chunks around spawn
        world.update_loaded_chunks(Point3::new(0.0, 0.0, 0.0));

        world
    }
    
    pub fn update_loaded_chunks(&mut self, player_pos: Point3<f32>) {
        let chunk_x = (player_pos.x as i32).div_euclid(Self::CHUNK_SIZE as i32);
        let chunk_z = (player_pos.z as i32).div_euclid(Self::CHUNK_SIZE as i32);

        // Always try to load chunks (progressively, limited per frame)
        self.load_chunks_around(chunk_x, chunk_z);
    }
    
    /// Force load chunks around a position (used for initial spawn)
    pub fn force_load_chunks_at(&mut self, pos: Point3<f32>) {
        let chunk_x = (pos.x as i32).div_euclid(Self::CHUNK_SIZE as i32);
        let chunk_z = (pos.z as i32).div_euclid(Self::CHUNK_SIZE as i32);
        self.load_chunks_around(chunk_x, chunk_z);
    }

    /// Force load ALL chunks within render distance synchronously (for initial loading)
    /// Returns progress as (loaded, total) for loading screen updates
    pub fn force_load_all_chunks(&mut self, pos: Point3<f32>) -> (usize, usize) {
        let chunk_x = (pos.x as i32).div_euclid(Self::CHUNK_SIZE as i32);
        let chunk_z = (pos.z as i32).div_euclid(Self::CHUNK_SIZE as i32);

        let mut total = 0;
        let mut loaded = 0;

        // Count total and load all chunks without rate limiting
        for x in (chunk_x - self.render_distance)..=(chunk_x + self.render_distance) {
            for z in (chunk_z - self.render_distance)..=(chunk_z + self.render_distance) {
                total += 1;
                let chunk_key = (x, z);
                if !self.chunks.contains_key(&chunk_key) {
                    self.load_chunk(x, z);
                }
                loaded += 1;
            }
        }

        self.player_chunk_pos = (chunk_x, chunk_z);
        (loaded, total)
    }

    /// Get the number of chunks that need to be loaded
    pub fn get_chunks_to_load_count(&self, pos: Point3<f32>) -> usize {
        let chunk_x = (pos.x as i32).div_euclid(Self::CHUNK_SIZE as i32);
        let chunk_z = (pos.z as i32).div_euclid(Self::CHUNK_SIZE as i32);

        let mut count = 0;
        for x in (chunk_x - self.render_distance)..=(chunk_x + self.render_distance) {
            for z in (chunk_z - self.render_distance)..=(chunk_z + self.render_distance) {
                if !self.chunks.contains_key(&(x, z)) {
                    count += 1;
                }
            }
        }
        count
    }
    
    fn load_chunks_around(&mut self, chunk_x: i32, chunk_z: i32) {
        // Limit chunks loaded per frame to prevent stuttering (1 is smoother than 2)
        const MAX_CHUNKS_TO_LOAD: usize = 1;
        const MAX_CHUNKS_TO_UNLOAD: usize = 2;

        self.player_chunk_pos = (chunk_x, chunk_z);

        // Find chunks that need loading, prioritizing closer chunks
        let mut chunks_to_load: Vec<(i32, i32, i32)> = Vec::new(); // (x, z, distance_sq)
        for x in (chunk_x - self.render_distance)..=(chunk_x + self.render_distance) {
            for z in (chunk_z - self.render_distance)..=(chunk_z + self.render_distance) {
                let chunk_key = (x, z);
                if !self.chunks.contains_key(&chunk_key) {
                    let dist_sq = (x - chunk_x).pow(2) + (z - chunk_z).pow(2);
                    chunks_to_load.push((x, z, dist_sq));
                }
            }
        }

        // Sort by distance (closest first) and load only a few per frame
        chunks_to_load.sort_by_key(|&(_, _, dist)| dist);
        for (x, z, _) in chunks_to_load.into_iter().take(MAX_CHUNKS_TO_LOAD) {
            self.load_chunk(x, z);
        }

        // Unload chunks outside render distance (limit per frame to prevent stuttering)
        let chunks_to_unload: Vec<(i32, i32)> = self.chunks.keys()
            .filter(|(x, z)| {
                let dx = (*x - chunk_x).abs();
                let dz = (*z - chunk_z).abs();
                dx > self.render_distance + 1 || dz > self.render_distance + 1
            })
            .take(MAX_CHUNKS_TO_UNLOAD)
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
        let temperature = self.temperature_noise.get([world_x * 0.003, world_z * 0.003]);
        let humidity = self.humidity_noise.get([world_x * 0.004, world_z * 0.004]);
        let elevation = self.get_base_continent_height(world_x, world_z);

        // High elevation = mountains regardless of other factors
        if elevation > 75.0 {
            return Biome::Mountains;
        }

        // Very low elevation = ocean
        if elevation < 40.0 {
            return Biome::Ocean;
        }

        // Biome based on temperature and humidity
        match (temperature, humidity) {
            (t, _) if t < -0.35 => Biome::Tundra,
            (t, h) if t > 0.35 && h < -0.1 => Biome::Desert,
            (_, h) if h > 0.25 => Biome::Forest,
            _ => Biome::Plains,
        }
    }

    // Multi-octave terrain height calculation
    fn get_terrain_height(&self, world_x: f64, world_z: f64) -> f64 {
        // Layer 1: Continental shapes (very large scale)
        let continent = self.continent_noise.get([world_x * 0.001, world_z * 0.001]);
        let continent_height = (continent + 1.0) * 0.5 * 30.0 + 35.0; // 35-65 base

        // Layer 2: Mountain ranges using ridged noise
        let mountain_mask = self.mountain_noise.get([world_x * 0.004, world_z * 0.004]);
        let mountain_mask = ((mountain_mask + 1.0) * 0.5).powf(2.0); // Concentrate mountains

        // Ridged noise for sharp mountain peaks
        let ridge1 = 1.0 - self.ridge_noise.get([world_x * 0.008, world_z * 0.008]).abs();
        let ridge2 = (1.0 - self.ridge_noise.get([world_x * 0.016, world_z * 0.016]).abs()) * 0.5;
        let ridged = (ridge1 + ridge2).powf(2.0) * 45.0 * mountain_mask; // Up to 45 blocks of mountains

        // Layer 3: Rolling hills
        let hills = self.hill_noise.get([world_x * 0.02, world_z * 0.02]);
        let hills = hills * 12.0; // ±12 blocks

        // Layer 4: Fine detail
        let detail1 = self.detail_noise.get([world_x * 0.05, world_z * 0.05]) * 4.0;
        let detail2 = self.detail_noise.get([world_x * 0.1, world_z * 0.1]) * 2.0;

        // Layer 5: Erosion (creates valleys and smooths terrain)
        let erosion = self.erosion_noise.get([world_x * 0.015, world_z * 0.015]);
        let erosion_factor = (erosion + 1.0) * 0.5; // 0-1
        let erosion_carve = if erosion < -0.3 { (erosion + 0.3) * 15.0 } else { 0.0 }; // Carve valleys

        // Combine all layers
        let mut height = continent_height + ridged + hills + detail1 + detail2 + erosion_carve;

        // Apply erosion smoothing to mountains
        height = height * (0.7 + 0.3 * erosion_factor);

        height.max(1.0).min((Self::CHUNK_HEIGHT - 5) as f64)
    }

    fn get_base_continent_height(&self, world_x: f64, world_z: f64) -> f64 {
        let continent = self.continent_noise.get([world_x * 0.001, world_z * 0.001]);
        let mountain_influence = self.mountain_noise.get([world_x * 0.004, world_z * 0.004]);
        (continent + 1.0) * 0.5 * 40.0 + 30.0 + mountain_influence.max(0.0) * 20.0
    }

    // Check if this location should have a river
    // Rivers only form at low elevations, close to sea level
    fn is_river(&self, world_x: f64, world_z: f64, terrain_height: f64) -> bool {
        // Rivers only exist in a narrow band above sea level
        let max_river_height = Self::SEA_LEVEL as f64 + 8.0;
        if terrain_height < Self::SEA_LEVEL as f64 + 2.0 || terrain_height > max_river_height {
            return false;
        }

        // River paths follow noise contours
        let river1 = self.river_noise.get([world_x * 0.008, world_z * 0.008]);
        let river2 = self.river_noise.get([world_x * 0.004 + 100.0, world_z * 0.004 + 100.0]);

        // Rivers form where noise is close to zero (creates paths)
        let river_threshold = 0.03;
        river1.abs() < river_threshold || river2.abs() < river_threshold * 1.5
    }

    // Check for lakes in depressions - only at low elevations
    fn is_lake(&self, world_x: f64, world_z: f64, terrain_height: f64) -> bool {
        // Lakes only form slightly above sea level, not in mountains
        let max_lake_height = Self::SEA_LEVEL as f64 + 10.0;
        if terrain_height < Self::SEA_LEVEL as f64 || terrain_height > max_lake_height {
            return false;
        }

        let lake = self.lake_noise.get([world_x * 0.02, world_z * 0.02]);
        let depression = self.erosion_noise.get([world_x * 0.015, world_z * 0.015]);

        // Lakes form in depressions where lake noise is high
        lake > 0.6 && depression < -0.25
    }

    // Get the water surface level for rivers/lakes (they fill to a consistent level)
    fn get_water_surface_level(&self, world_x: f64, world_z: f64, terrain_height: f64) -> Option<usize> {
        if self.is_river(world_x, world_z, terrain_height) {
            // Rivers fill to slightly above sea level
            return Some(Self::SEA_LEVEL + 3);
        }
        if self.is_lake(world_x, world_z, terrain_height) {
            // Lakes fill to a consistent level based on local depression
            return Some(Self::SEA_LEVEL + 5);
        }
        None
    }
    
    fn generate_chunk_data(&self, chunk_x: i32, chunk_z: i32) -> Chunk {
        let mut blocks = vec![vec![vec![BlockType::Air; Self::CHUNK_SIZE]; Self::CHUNK_HEIGHT]; Self::CHUNK_SIZE];

        for x in 0..Self::CHUNK_SIZE {
            for z in 0..Self::CHUNK_SIZE {
                let world_x = (chunk_x * Self::CHUNK_SIZE as i32 + x as i32) as f64;
                let world_z = (chunk_z * Self::CHUNK_SIZE as i32 + z as i32) as f64;

                let biome = self.get_biome(world_x, world_z);
                let raw_height = self.get_terrain_height(world_x, world_z);
                let terrain_height = raw_height as usize;

                // Check for water features (rivers, lakes) - they have a fixed water level
                let water_surface = self.get_water_surface_level(world_x, world_z, raw_height);

                // For rivers/lakes, if terrain is above water level, carve down to water level
                // If terrain is below water level, it becomes the lake/river bed
                let is_water_feature = water_surface.is_some();
                let water_level = water_surface.unwrap_or(0);

                // Determine solid ground height
                let solid_height = if is_water_feature && terrain_height > water_level {
                    // Carve terrain down to below water level for river/lake bed
                    water_level.saturating_sub(2)
                } else {
                    terrain_height
                };

                // Generate cave system
                let cave_blocks = self.generate_caves(world_x, world_z, solid_height);

                for y in 0..Self::CHUNK_HEIGHT {
                    // Bedrock at bottom
                    if y == 0 {
                        blocks[x][y][z] = BlockType::Stone;
                        continue;
                    }

                    // Check for caves (but not too close to surface)
                    if cave_blocks.contains(&y) && y < solid_height.saturating_sub(4) {
                        blocks[x][y][z] = BlockType::Air;
                        continue;
                    }

                    if y <= solid_height {
                        // Generate ores in deep stone
                        if y < solid_height.saturating_sub(8) {
                            if let Some(ore) = self.generate_ore(world_x, y as f64, world_z) {
                                blocks[x][y][z] = ore;
                                continue;
                            }
                        }

                        // Terrain layers
                        blocks[x][y][z] = if y == solid_height {
                            self.get_surface_block(biome, solid_height, world_x, world_z, is_water_feature)
                        } else if y > solid_height.saturating_sub(4) {
                            self.get_subsurface_block(biome, solid_height, y)
                        } else {
                            BlockType::Stone
                        };
                    } else if is_water_feature && y <= water_level {
                        // River/lake water - fills up to the fixed water level
                        blocks[x][y][z] = if biome == Biome::Tundra { BlockType::Ice } else { BlockType::Water };
                    } else if y <= Self::SEA_LEVEL && terrain_height < Self::SEA_LEVEL {
                        // Ocean water
                        blocks[x][y][z] = if biome == Biome::Tundra { BlockType::Ice } else { BlockType::Water };
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
        let current_height = self.get_terrain_height(world_x, world_z);

        // Check surrounding positions for water
        for dx in -3..=3 {
            for dz in -3..=3 {
                if dx == 0 && dz == 0 { continue; }

                let check_x = world_x + dx as f64;
                let check_z = world_z + dz as f64;
                let check_height = self.get_terrain_height(check_x, check_z);

                // Near ocean or river/lake
                if check_height < Self::SEA_LEVEL as f64 && current_height >= Self::SEA_LEVEL as f64 {
                    return true;
                }
                if self.is_river(check_x, check_z, check_height) || self.is_lake(check_x, check_z, check_height) {
                    return true;
                }
            }
        }
        false
    }

    fn get_surface_block(&self, biome: Biome, height: usize, world_x: f64, world_z: f64, is_water_bottom: bool) -> BlockType {
        // Underwater surfaces
        if is_water_bottom || height < Self::SEA_LEVEL {
            let depth_noise = self.detail_noise.get([world_x * 0.1, world_z * 0.1]);
            return if depth_noise > 0.3 {
                BlockType::Gravel
            } else if depth_noise < -0.3 {
                BlockType::Clay
            } else {
                BlockType::Sand
            };
        }

        // Beach/shore areas
        if height <= Self::BEACH_HEIGHT && self.is_near_water(world_x, world_z) {
            return BlockType::Sand;
        }

        match biome {
            Biome::Plains | Biome::Forest => BlockType::Grass,
            Biome::Desert => BlockType::Sand,
            Biome::Mountains => {
                if height > 85 {
                    BlockType::Snow
                } else if height > 70 {
                    // Rocky mountain tops
                    let rock_noise = self.detail_noise.get([world_x * 0.2, world_z * 0.2]);
                    if rock_noise > 0.2 { BlockType::Stone } else { BlockType::Grass }
                } else {
                    BlockType::Grass
                }
            },
            Biome::Tundra => {
                let snow_noise = self.detail_noise.get([world_x * 0.15, world_z * 0.15]);
                if snow_noise > -0.2 { BlockType::Snow } else { BlockType::Grass }
            },
            Biome::Ocean => BlockType::Sand,
        }
    }

    fn get_subsurface_block(&self, biome: Biome, surface_height: usize, current_y: usize) -> BlockType {
        let depth = surface_height - current_y;

        match biome {
            Biome::Desert | Biome::Ocean => {
                if depth < 4 { BlockType::Sand } else { BlockType::Stone }
            },
            Biome::Mountains => {
                if depth < 2 { BlockType::Dirt } else { BlockType::Stone }
            },
            _ => {
                if depth < 4 { BlockType::Dirt } else { BlockType::Stone }
            },
        }
    }

    fn generate_caves(&self, world_x: f64, world_z: f64, max_height: usize) -> Vec<usize> {
        let mut cave_levels = Vec::new();

        // Generate multi-level cave systems
        for y in 5..max_height.saturating_sub(10) {
            let y_f = y as f64;

            // 3D cave noise for more organic shapes
            let cave1 = self.cave_noise.get([world_x * 0.03, y_f * 0.03, world_z * 0.03]);
            let cave2 = self.cave_noise.get([world_x * 0.06, y_f * 0.06, world_z * 0.06]);

            // Larger caverns
            let cavern = self.cave_noise.get([world_x * 0.015, y_f * 0.02, world_z * 0.015]);

            // Depth factor - more caves deeper down
            let depth_factor = 1.0 - (y as f64 / max_height as f64);
            let cave_threshold = 0.55 - depth_factor * 0.1;

            // Combine noise for cave generation
            let combined = (cave1 + cave2 * 0.5) / 1.5;

            if combined > cave_threshold || cavern > 0.65 {
                cave_levels.push(y);
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
        // Prefer spawning on grass in plains/forest biomes
        for radius in 0i32..50 {
            for dx in -radius..=radius {
                for dz in -radius..=radius {
                    if radius > 0 && dx.abs() < radius && dz.abs() < radius {
                        continue;
                    }

                    let x = dx;
                    let z = dz;

                    // Check biome - prefer non-ocean, non-mountain spawns
                    let biome = self.get_biome(x as f64, z as f64);
                    if biome == Biome::Ocean {
                        continue;
                    }

                    for y in (1..Self::CHUNK_HEIGHT as i32 - 4).rev() {
                        if let Some(block) = self.get_block(x, y, z) {
                            let is_solid_ground = matches!(block,
                                BlockType::Grass | BlockType::Dirt | BlockType::Stone |
                                BlockType::Sand | BlockType::Snow | BlockType::Cobblestone
                            );

                            if is_solid_ground {
                                let mut has_clearance = true;
                                for check_y in 1..=3 {
                                    if let Some(above) = self.get_block(x, y + check_y, z) {
                                        if above != BlockType::Air {
                                            has_clearance = false;
                                            break;
                                        }
                                    } else {
                                        has_clearance = false;
                                        break;
                                    }
                                }

                                if has_clearance && y > Self::SEA_LEVEL as i32 {
                                    return Point3::new(
                                        x as f32 + 0.5,
                                        y as f32 + 1.0 + 1.8,
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
        Point3::new(0.5, 60.0, 0.5)
    }
}