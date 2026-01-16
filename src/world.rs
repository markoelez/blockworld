use noise::{NoiseFn, Perlin, Simplex};
use cgmath::{Vector3, Point3};
use std::collections::{HashMap, VecDeque};
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
    Torch,  // Emits light
    Chest,  // Storage container
    // New block types for world generation
    Lava,              // Emissive, damages player
    MobSpawner,        // Dungeon spawner block
    Rail,              // Mineshaft decoration
    Planks,            // Wooden planks
    Fence,             // Support beams
    Brick,             // Stone brick
    MossyCobblestone,  // Aged dungeon walls
    // Food items (dropped by animals)
    RawPork,           // Dropped by pigs, +3 hunger
    RawBeef,           // Dropped by cows, +3 hunger
    RawChicken,        // Dropped by chickens, +2 hunger
    RawMutton,         // Dropped by sheep, +2 hunger
}

impl BlockType {
    /// Returns (hunger_restore, saturation_restore) if this is a food item
    pub fn food_properties(&self) -> Option<(f32, f32)> {
        match self {
            BlockType::RawPork => Some((3.0, 1.8)),
            BlockType::RawBeef => Some((3.0, 1.8)),
            BlockType::RawChicken => Some((2.0, 1.2)),
            BlockType::RawMutton => Some((2.0, 1.2)),
            _ => None,
        }
    }

    /// Check if this block type is food
    pub fn is_food(&self) -> bool {
        self.food_properties().is_some()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TorchFace {
    Top,    // Sitting on top of a block
    North,  // Torch tilts toward -Z (placed on +Z face of solid block)
    South,  // Torch tilts toward +Z (placed on -Z face of solid block)
    East,   // Torch tilts toward +X (placed on -X face of solid block)
    West,   // Torch tilts toward -X (placed on +X face of solid block)
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
    // New noise for world generation enhancements
    lava_noise: Perlin,           // Lava pool locations
    ore_vein_noise: Perlin,       // Ore vein shapes
    dungeon_noise: Perlin,        // Dungeon placement
    mineshaft_noise: Perlin,      // Mineshaft corridors
    cavern_noise: Perlin,         // Large cavern shapes
    render_distance: i32,
    player_chunk_pos: (i32, i32),
    block_damage: HashMap<(i32, i32, i32), f32>,
    pub torch_orientations: HashMap<(i32, i32, i32), TorchFace>,
    pub chest_contents: HashMap<(i32, i32, i32), [Option<(BlockType, u32)>; 9]>,
    // Water flow system: level 8 = source, 7-1 = flowing (7 = nearly full, 1 = thin layer)
    pub water_levels: HashMap<(i32, i32, i32), u8>,
    water_update_queue: VecDeque<(i32, i32, i32)>,
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
            // New noise for world generation enhancements
            lava_noise: Perlin::new(seed.wrapping_add(14)),
            ore_vein_noise: Perlin::new(seed.wrapping_add(15)),
            dungeon_noise: Perlin::new(seed.wrapping_add(16)),
            mineshaft_noise: Perlin::new(seed.wrapping_add(17)),
            cavern_noise: Perlin::new(seed.wrapping_add(18)),
            render_distance: 6,
            player_chunk_pos: (0, 0),
            block_damage: HashMap::new(),
            torch_orientations: HashMap::new(),
            chest_contents: HashMap::new(),
            water_levels: HashMap::new(),
            water_update_queue: VecDeque::new(),
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

        // Generate underground structures first
        self.generate_dungeons_for_chunk(&mut chunk);
        self.generate_mineshafts_for_chunk(&mut chunk);

        // Generate surface structures and trees
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
    
    pub fn get_biome(&self, world_x: f64, world_z: f64) -> Biome {
        let temperature = self.temperature_noise.get([world_x * 0.003, world_z * 0.003]);
        let humidity = self.humidity_noise.get([world_x * 0.004, world_z * 0.004]);
        let elevation = self.get_base_continent_height(world_x, world_z);

        // High elevation = mountains regardless of other factors
        if elevation > 70.0 {
            return Biome::Mountains;
        }

        // Very low elevation = ocean
        if elevation < 38.0 {
            return Biome::Ocean;
        }

        // Biome based on temperature and humidity
        // Adjusted thresholds to make plains more common
        match (temperature, humidity) {
            (t, _) if t < -0.5 => Biome::Tundra,
            (t, h) if t > 0.5 && h < -0.2 => Biome::Desert,
            (_, h) if h > 0.4 => Biome::Forest,
            _ => Biome::Plains,
        }
    }

    /// Check if a world position is a village location (for NPC spawning)
    pub fn is_village_location(&self, world_x: f64, world_z: f64) -> bool {
        // Spawn villagers in any non-ocean, non-mountain biome
        let biome = self.get_biome(world_x, world_z);
        if biome == Biome::Ocean || biome == Biome::Mountains {
            return false;
        }

        // Use tree_noise for village detection - lowered threshold significantly
        let structure_noise = self.tree_noise.get([world_x * 0.015, world_z * 0.015]);
        structure_noise > 0.5  // Very common villages for testing
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
        // Range: 35-65 base + 0-10 mountain = 35-75 total
        // This gives mostly plains (40-80 range) with some ocean and mountains
        (continent + 1.0) * 0.5 * 30.0 + 35.0 + mountain_influence.max(0.0) * 10.0
    }

    // Check if neighboring terrain can contain water at the given level
    // Uses a larger radius to prevent floating water
    fn can_contain_water(&self, world_x: f64, world_z: f64, water_level: usize) -> bool {
        // Check a wider area - water can only exist if ALL nearby terrain can hold it
        const CHECK_RADIUS: i32 = 8;

        for dx in -CHECK_RADIUS..=CHECK_RADIUS {
            for dz in -CHECK_RADIUS..=CHECK_RADIUS {
                if dx == 0 && dz == 0 { continue; }

                let nx = world_x + dx as f64;
                let nz = world_z + dz as f64;
                let neighbor_height = self.get_terrain_height(nx, nz);

                // If any neighbor is lower than water level and not a water feature, water would flow out
                if neighbor_height < water_level as f64 {
                    let neighbor_is_water = self.is_river_raw(nx, nz) || self.is_lake_raw(nx, nz);
                    if !neighbor_is_water {
                        return false;
                    }
                }
            }
        }
        true
    }

    // Height range constants for water features - keep close to sea level for realism
    const RIVER_MIN_HEIGHT: f64 = Self::SEA_LEVEL as f64 - 2.0;
    const RIVER_MAX_HEIGHT: f64 = Self::SEA_LEVEL as f64 + 2.0;
    const LAKE_MIN_HEIGHT: f64 = Self::SEA_LEVEL as f64 - 3.0;
    const LAKE_MAX_HEIGHT: f64 = Self::SEA_LEVEL as f64 + 1.0;
    const RIVER_THRESHOLD: f64 = 0.03;

    // Check if terrain height is valid for a river
    fn is_river_height_valid(terrain_height: f64) -> bool {
        terrain_height >= Self::RIVER_MIN_HEIGHT && terrain_height <= Self::RIVER_MAX_HEIGHT
    }

    // Check if terrain height is valid for a lake
    fn is_lake_height_valid(terrain_height: f64) -> bool {
        terrain_height >= Self::LAKE_MIN_HEIGHT && terrain_height <= Self::LAKE_MAX_HEIGHT
    }

    // Check river noise pattern (whether position lies on a river path)
    fn is_river_path(&self, world_x: f64, world_z: f64) -> bool {
        let river1 = self.river_noise.get([world_x * 0.008, world_z * 0.008]);
        let river2 = self.river_noise.get([world_x * 0.004 + 100.0, world_z * 0.004 + 100.0]);
        river1.abs() < Self::RIVER_THRESHOLD || river2.abs() < Self::RIVER_THRESHOLD * 1.5
    }

    // Check lake noise pattern (whether position lies in a lake depression)
    fn is_lake_depression(&self, world_x: f64, world_z: f64) -> bool {
        let lake = self.lake_noise.get([world_x * 0.02, world_z * 0.02]);
        let depression = self.erosion_noise.get([world_x * 0.015, world_z * 0.015]);
        lake > 0.6 && depression < -0.25
    }

    // Raw river check without containment validation (to avoid recursion)
    fn is_river_raw(&self, world_x: f64, world_z: f64) -> bool {
        let terrain_height = self.get_terrain_height(world_x, world_z);
        Self::is_river_height_valid(terrain_height) && self.is_river_path(world_x, world_z)
    }

    // Raw lake check without containment validation (to avoid recursion)
    fn is_lake_raw(&self, world_x: f64, world_z: f64) -> bool {
        let terrain_height = self.get_terrain_height(world_x, world_z);
        Self::is_lake_height_valid(terrain_height) && self.is_lake_depression(world_x, world_z)
    }

    // Check if this location should have a river (with containment validation)
    fn is_river(&self, world_x: f64, world_z: f64, terrain_height: f64) -> bool {
        Self::is_river_height_valid(terrain_height)
            && self.is_river_path(world_x, world_z)
            && self.can_contain_water(world_x, world_z, Self::SEA_LEVEL)
    }

    // Check for lakes in depressions (with containment validation)
    fn is_lake(&self, world_x: f64, world_z: f64, terrain_height: f64) -> bool {
        Self::is_lake_height_valid(terrain_height)
            && self.is_lake_depression(world_x, world_z)
            && self.can_contain_water(world_x, world_z, Self::SEA_LEVEL)
    }

    // Get the water surface level for rivers/lakes (they fill to sea level)
    fn get_water_surface_level(&self, world_x: f64, world_z: f64, terrain_height: f64) -> Option<usize> {
        if self.is_river(world_x, world_z, terrain_height) {
            return Some(Self::SEA_LEVEL);
        }
        if self.is_lake(world_x, world_z, terrain_height) {
            return Some(Self::SEA_LEVEL);
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
                        // Deep caves can have lava pools instead of air
                        if self.should_have_lava(world_x, y as f64, world_z) {
                            // Only place lava if there's solid ground below (no floating lava)
                            if y > 1 && !cave_blocks.contains(&(y - 1)) {
                                blocks[x][y][z] = BlockType::Lava;
                            } else {
                                blocks[x][y][z] = BlockType::Air;
                            }
                        } else {
                            blocks[x][y][z] = BlockType::Air;
                        }
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

            // 3D cave noise for more organic shapes - worm-like tunnels
            let cave1 = self.cave_noise.get([world_x * 0.03, y_f * 0.03, world_z * 0.03]);
            let cave2 = self.cave_noise.get([world_x * 0.06, y_f * 0.06, world_z * 0.06]);

            // Large caverns using dedicated cavern noise - lower frequency for bigger spaces
            let cavern = self.cavern_noise.get([world_x * 0.008, y_f * 0.012, world_z * 0.008]);

            // Secondary cavern layer for variety with offset
            let cavern2 = self.cavern_noise.get([
                (world_x + 1000.0) * 0.01,
                y_f * 0.015,
                (world_z + 1000.0) * 0.01
            ]);

            // Depth factor - more caves and larger caverns deeper down
            let depth_factor = 1.0 - (y as f64 / max_height as f64);
            let cave_threshold = 0.55 - depth_factor * 0.1;

            // Cavern threshold scales with depth (bigger caverns deeper)
            let cavern_threshold = 0.55 - depth_factor * 0.15;

            // Combine noise for cave generation
            let combined = (cave1 + cave2 * 0.5) / 1.5;

            // Carve if tunnel noise OR large cavern noise exceeds threshold
            if combined > cave_threshold
                || cavern > cavern_threshold
                || (cavern2 > 0.6 && depth_factor > 0.5)
            {
                cave_levels.push(y);
            }
        }

        cave_levels
    }

    /// Check if a position should have lava (deep underground in caves)
    fn should_have_lava(&self, world_x: f64, world_y: f64, world_z: f64) -> bool {
        const LAVA_MAX_HEIGHT: usize = 15;  // Lava only below y=15

        if world_y as usize > LAVA_MAX_HEIGHT {
            return false;
        }

        // Use lava noise to create pools
        let lava_val = self.lava_noise.get([world_x * 0.02, world_y * 0.03, world_z * 0.02]);
        let pool_val = self.lava_noise.get([world_x * 0.05, world_y * 0.01, world_z * 0.05]);

        lava_val > 0.6 && pool_val > 0.4
    }
    
    fn generate_ore(&self, world_x: f64, world_y: f64, world_z: f64) -> Option<BlockType> {
        let depth_factor = (Self::CHUNK_HEIGHT as f64 - world_y) / Self::CHUNK_HEIGHT as f64;

        // Ore vein seed noise - determines vein center locations
        let vein_seed = self.ore_noise.get([world_x * 0.08, world_y * 0.08, world_z * 0.08]);

        // Ore vein spread noise - determines vein shape/extent
        let vein_spread = self.ore_vein_noise.get([world_x * 0.15, world_y * 0.15, world_z * 0.15]);

        // Combined vein value - higher means more likely to be in a vein
        let vein_value = (vein_seed + vein_spread * 0.7) / 1.7;

        // Determine ore type based on depth and seed noise
        // Veins are larger for common ores, smaller for rare ones
        let (ore_type, threshold, vein_threshold) = if depth_factor > 0.85 && vein_seed > 0.88 {
            // Diamond: rare, small veins (3-5 blocks)
            (Some(BlockType::Diamond), 0.88, 0.82)
        } else if depth_factor > 0.65 && vein_seed > 0.78 {
            // Gold: uncommon, small-medium veins (4-7 blocks)
            (Some(BlockType::Gold), 0.78, 0.75)
        } else if depth_factor > 0.40 && vein_seed > 0.68 {
            // Iron: common, medium veins (6-12 blocks)
            (Some(BlockType::Iron), 0.68, 0.60)
        } else if depth_factor > 0.20 && vein_seed > 0.55 {
            // Coal: very common, large veins (10-20 blocks)
            (Some(BlockType::Coal), 0.55, 0.45)
        } else {
            (None, 1.0, 1.0)
        };

        // Check if this block is part of a vein
        if let Some(ore) = ore_type {
            // Block is part of vein if:
            // 1. It's at a vein seed point (high ore_noise), OR
            // 2. It's near a vein seed and within the vein spread
            if vein_seed > threshold || vein_value > vein_threshold {
                return Some(ore);
            }
        }

        None
    }

    /// Generate dungeon structures in caves
    fn generate_dungeons_for_chunk(&self, chunk: &mut Chunk) {
        let chunk_world_x = chunk.position.x * Self::CHUNK_SIZE as i32;
        let chunk_world_z = chunk.position.z * Self::CHUNK_SIZE as i32;

        // Check if this chunk should have a dungeon (~3% of chunks)
        let dungeon_seed = self.dungeon_noise.get([
            chunk_world_x as f64 * 0.1,
            chunk_world_z as f64 * 0.1
        ]);

        if dungeon_seed < 0.85 {
            return;
        }

        // Find a suitable location for the dungeon (cave space at y=15-40)
        for attempt in 0..5 {
            let x = ((dungeon_seed * 1000.0 + attempt as f64 * 137.0) as usize) % (Self::CHUNK_SIZE - 6) + 3;
            let z = ((dungeon_seed * 2000.0 + attempt as f64 * 173.0) as usize) % (Self::CHUNK_SIZE - 6) + 3;

            // Search for a cave space
            for y in (15..40).rev() {
                if y + 5 >= Self::CHUNK_HEIGHT { continue; }
                if x + 5 >= Self::CHUNK_SIZE || z + 5 >= Self::CHUNK_SIZE { continue; }

                // Check if there's enough air space for a dungeon
                let mut has_space = true;
                let mut has_floor = false;

                for dx in 0..5 {
                    for dz in 0..5 {
                        // Need air at this level and above
                        if chunk.blocks[x + dx][y][z + dz] != BlockType::Air ||
                           chunk.blocks[x + dx][y + 1][z + dz] != BlockType::Air ||
                           chunk.blocks[x + dx][y + 2][z + dz] != BlockType::Air {
                            has_space = false;
                            break;
                        }
                        // Need solid floor below
                        if y > 0 && chunk.blocks[x + dx][y - 1][z + dz] == BlockType::Stone {
                            has_floor = true;
                        }
                    }
                    if !has_space { break; }
                }

                if has_space && has_floor {
                    self.place_dungeon_in_chunk(chunk, x, y, z, chunk_world_x, chunk_world_z);
                    return;
                }
            }
        }
    }

    fn place_dungeon_in_chunk(&self, chunk: &mut Chunk, x: usize, y: usize, z: usize, chunk_world_x: i32, chunk_world_z: i32) {
        let width = 5;
        let height = 4;
        let depth = 5;
        let mut rng = rand::thread_rng();

        // Build dungeon walls, floor, and ceiling
        for dx in 0..width {
            for dy in 0..height {
                for dz in 0..depth {
                    let bx = x + dx;
                    let by = y + dy;
                    let bz = z + dz;

                    if bx >= Self::CHUNK_SIZE || by >= Self::CHUNK_HEIGHT || bz >= Self::CHUNK_SIZE {
                        continue;
                    }

                    let is_wall = dx == 0 || dx == width - 1 || dz == 0 || dz == depth - 1;
                    let is_floor = dy == 0;
                    let is_ceiling = dy == height - 1;

                    if is_floor || is_ceiling || is_wall {
                        // Mix cobblestone and mossy cobblestone for aged look
                        chunk.blocks[bx][by][bz] = if rng.gen::<f32>() < 0.3 {
                            BlockType::MossyCobblestone
                        } else {
                            BlockType::Cobblestone
                        };
                    } else {
                        chunk.blocks[bx][by][bz] = BlockType::Air;
                    }
                }
            }
        }

        // Place mob spawner in center
        let spawner_x = x + width / 2;
        let spawner_y = y + 1;
        let spawner_z = z + depth / 2;
        if spawner_x < Self::CHUNK_SIZE && spawner_y < Self::CHUNK_HEIGHT && spawner_z < Self::CHUNK_SIZE {
            chunk.blocks[spawner_x][spawner_y][spawner_z] = BlockType::MobSpawner;
        }

        // Place chest in corner
        let chest_x = x + 1;
        let chest_y = y + 1;
        let chest_z = z + 1;
        if chest_x < Self::CHUNK_SIZE && chest_y < Self::CHUNK_HEIGHT && chest_z < Self::CHUNK_SIZE {
            chunk.blocks[chest_x][chest_y][chest_z] = BlockType::Chest;
            // Populate chest with dungeon loot
            let world_x = chunk_world_x + chest_x as i32;
            let world_z = chunk_world_z + chest_z as i32;
            self.populate_dungeon_chest(world_x, chest_y as i32, world_z);
        }
    }

    fn populate_dungeon_chest(&self, x: i32, y: i32, z: i32) {
        // Note: This creates chest contents but they won't persist since we don't
        // have mutable access to World here. This would need to be called after
        // chunk generation with proper world access. For now, skip.
        // In a full implementation, this would add items like:
        // - Coal (1-8), Iron (1-4), Gold (1-3), Diamond (1-2)
        // - Cobblestone, Torches
    }

    /// Generate mineshaft tunnels
    fn generate_mineshafts_for_chunk(&self, chunk: &mut Chunk) {
        let chunk_world_x = chunk.position.x * Self::CHUNK_SIZE as i32;
        let chunk_world_z = chunk.position.z * Self::CHUNK_SIZE as i32;

        // Check if this chunk should have a mineshaft (~2% of chunks)
        let shaft_seed = self.mineshaft_noise.get([
            chunk_world_x as f64 * 0.08,
            chunk_world_z as f64 * 0.08
        ]);

        if shaft_seed < 0.90 {
            return;
        }

        // Mineshaft Y level (20-50)
        let base_y = 25 + ((shaft_seed * 1000.0) as usize % 20);

        // Generate main corridor and branches
        let mut rng = rand::thread_rng();

        // Main corridor direction (0=X, 1=Z)
        let main_dir = if shaft_seed > 0.95 { 0 } else { 1 };

        // Place main corridor through chunk
        if main_dir == 0 {
            // X-direction corridor
            let z = Self::CHUNK_SIZE / 2;
            for x in 0..Self::CHUNK_SIZE {
                self.place_mineshaft_segment(chunk, x, base_y, z, x % 4 == 0);

                // Branch corridors
                if x % 6 == 3 && rng.gen::<f32>() < 0.4 {
                    let branch_len = rng.gen_range(4..8);
                    for dz in 1..=branch_len {
                        if z + dz < Self::CHUNK_SIZE {
                            self.place_mineshaft_segment(chunk, x, base_y, z + dz, dz % 4 == 0);
                        }
                        if z >= dz {
                            self.place_mineshaft_segment(chunk, x, base_y, z - dz, dz % 4 == 0);
                        }
                    }
                }
            }
        } else {
            // Z-direction corridor
            let x = Self::CHUNK_SIZE / 2;
            for z in 0..Self::CHUNK_SIZE {
                self.place_mineshaft_segment(chunk, x, base_y, z, z % 4 == 0);

                // Branch corridors
                if z % 6 == 3 && rng.gen::<f32>() < 0.4 {
                    let branch_len = rng.gen_range(4..8);
                    for dx in 1..=branch_len {
                        if x + dx < Self::CHUNK_SIZE {
                            self.place_mineshaft_segment(chunk, x + dx, base_y, z, dx % 4 == 0);
                        }
                        if x >= dx {
                            self.place_mineshaft_segment(chunk, x - dx, base_y, z, dx % 4 == 0);
                        }
                    }
                }
            }
        }
    }

    fn place_mineshaft_segment(&self, chunk: &mut Chunk, x: usize, y: usize, z: usize, has_support: bool) {
        if x >= Self::CHUNK_SIZE || z >= Self::CHUNK_SIZE || y + 3 >= Self::CHUNK_HEIGHT {
            return;
        }

        // Carve 3-wide, 3-tall tunnel
        for dx in 0..=2 {
            for dy in 0..=2 {
                let bx = if x > 0 { x - 1 + dx } else { dx };
                if bx >= Self::CHUNK_SIZE { continue; }

                // Floor is planks
                if dy == 0 {
                    chunk.blocks[bx][y][z] = BlockType::Planks;
                } else {
                    chunk.blocks[bx][y + dy][z] = BlockType::Air;
                }
            }
        }

        // Add support beams
        if has_support && x > 0 && x < Self::CHUNK_SIZE - 1 {
            // Left fence post
            chunk.blocks[x - 1][y + 1][z] = BlockType::Fence;
            chunk.blocks[x - 1][y + 2][z] = BlockType::Fence;
            // Right fence post
            if x + 1 < Self::CHUNK_SIZE {
                chunk.blocks[x + 1][y + 1][z] = BlockType::Fence;
                chunk.blocks[x + 1][y + 2][z] = BlockType::Fence;
            }
            // Top beam
            for dx in 0..=2 {
                let bx = if x > 0 { x - 1 + dx } else { dx };
                if bx < Self::CHUNK_SIZE && y + 3 < Self::CHUNK_HEIGHT {
                    chunk.blocks[bx][y + 3][z] = BlockType::Planks;
                }
            }
        }

        // Rail in center
        chunk.blocks[x][y + 1][z] = BlockType::Rail;
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
        // Use noise to determine building type
        let world_x = chunk.position.x * Self::CHUNK_SIZE as i32 + local_x as i32;
        let world_z = chunk.position.z * Self::CHUNK_SIZE as i32 + local_z as i32;
        let building_selector = self.tree_noise.get([world_x as f64 * 0.1, world_z as f64 * 0.1]);

        // Select building type based on noise
        if building_selector > 0.6 {
            self.place_village_well(chunk, local_x, local_z, ground_y);
        } else if building_selector > 0.3 {
            self.place_village_large_house(chunk, local_x, local_z, ground_y);
        } else if building_selector > 0.0 {
            self.place_village_blacksmith(chunk, local_x, local_z, ground_y);
        } else if building_selector > -0.3 {
            self.place_village_church(chunk, local_x, local_z, ground_y);
        } else if building_selector > -0.6 {
            self.place_village_farm(chunk, local_x, local_z, ground_y);
        } else {
            self.place_village_small_house(chunk, local_x, local_z, ground_y);
        }

        // Place gravel paths around the structure
        self.place_village_paths(chunk, local_x, local_z, ground_y);
    }

    fn place_village_small_house(&self, chunk: &mut Chunk, local_x: usize, local_z: usize, ground_y: usize) {
        // 5x5 wooden house with door and windows
        for dx in -2..=2 {
            for dz in -2..=2 {
                for dy in 0..=4 {
                    let x = local_x as i32 + dx;
                    let z = local_z as i32 + dz;
                    let y = ground_y + dy;

                    if x < 0 || x >= Self::CHUNK_SIZE as i32 ||
                       z < 0 || z >= Self::CHUNK_SIZE as i32 ||
                       y >= Self::CHUNK_HEIGHT {
                        continue;
                    }

                    let is_edge = dx.abs() == 2 || dz.abs() == 2;
                    let is_corner = dx.abs() == 2 && dz.abs() == 2;

                    if dy == 0 {
                        // Floor
                        chunk.blocks[x as usize][y][z as usize] = BlockType::Planks;
                    } else if dy <= 3 && is_edge && !is_corner {
                        // Walls
                        if dy == 2 && (dx == 0 || dz == 0) && !is_corner {
                            // Windows (glass)
                            chunk.blocks[x as usize][y][z as usize] = BlockType::Ice;
                        } else if dy == 1 && dx == 0 && dz == 2 {
                            // Door opening (air)
                            chunk.blocks[x as usize][y][z as usize] = BlockType::Air;
                        } else {
                            chunk.blocks[x as usize][y][z as usize] = BlockType::Planks;
                        }
                    } else if dy <= 3 && is_corner {
                        // Corner posts
                        chunk.blocks[x as usize][y][z as usize] = BlockType::Wood;
                    } else if dy == 4 && (dx.abs() <= 2 && dz.abs() <= 2) {
                        // Roof
                        chunk.blocks[x as usize][y][z as usize] = BlockType::Brick;
                    } else if dy <= 3 && !is_edge {
                        // Interior air
                        chunk.blocks[x as usize][y][z as usize] = BlockType::Air;
                    }
                }
            }
        }
    }

    fn place_village_large_house(&self, chunk: &mut Chunk, local_x: usize, local_z: usize, ground_y: usize) {
        // 7x7 two-story house
        for dx in -3..=3 {
            for dz in -3..=3 {
                for dy in 0..=6 {
                    let x = local_x as i32 + dx;
                    let z = local_z as i32 + dz;
                    let y = ground_y + dy;

                    if x < 0 || x >= Self::CHUNK_SIZE as i32 ||
                       z < 0 || z >= Self::CHUNK_SIZE as i32 ||
                       y >= Self::CHUNK_HEIGHT {
                        continue;
                    }

                    let is_edge = dx.abs() == 3 || dz.abs() == 3;
                    let is_corner = dx.abs() == 3 && dz.abs() == 3;

                    if dy == 0 {
                        // Ground floor
                        chunk.blocks[x as usize][y][z as usize] = BlockType::Cobblestone;
                    } else if dy == 3 && !is_edge {
                        // Second floor
                        chunk.blocks[x as usize][y][z as usize] = BlockType::Planks;
                    } else if dy <= 5 && is_edge && !is_corner {
                        // Walls
                        if (dy == 2 || dy == 5) && (dx == 0 || dz == 0) {
                            chunk.blocks[x as usize][y][z as usize] = BlockType::Ice;
                        } else if dy == 1 && dx == 0 && dz == 3 {
                            // Door
                            chunk.blocks[x as usize][y][z as usize] = BlockType::Air;
                        } else {
                            chunk.blocks[x as usize][y][z as usize] = BlockType::Planks;
                        }
                    } else if dy <= 5 && is_corner {
                        // Corner supports
                        chunk.blocks[x as usize][y][z as usize] = BlockType::Wood;
                    } else if dy == 6 {
                        // Pitched roof
                        if dx.abs() <= 2 && dz.abs() <= 2 {
                            chunk.blocks[x as usize][y][z as usize] = BlockType::Brick;
                        }
                    } else if dy <= 5 && !is_edge {
                        chunk.blocks[x as usize][y][z as usize] = BlockType::Air;
                    }
                }
            }
        }
    }

    fn place_village_blacksmith(&self, chunk: &mut Chunk, local_x: usize, local_z: usize, ground_y: usize) {
        // 7x7 stone blacksmith with furnace area
        for dx in -3..=3 {
            for dz in -3..=3 {
                for dy in 0..=4 {
                    let x = local_x as i32 + dx;
                    let z = local_z as i32 + dz;
                    let y = ground_y + dy;

                    if x < 0 || x >= Self::CHUNK_SIZE as i32 ||
                       z < 0 || z >= Self::CHUNK_SIZE as i32 ||
                       y >= Self::CHUNK_HEIGHT {
                        continue;
                    }

                    let is_edge = dx.abs() == 3 || dz.abs() == 3;

                    if dy == 0 {
                        chunk.blocks[x as usize][y][z as usize] = BlockType::Cobblestone;
                    } else if dy <= 3 && is_edge {
                        chunk.blocks[x as usize][y][z as usize] = BlockType::Cobblestone;
                    } else if dy == 4 {
                        chunk.blocks[x as usize][y][z as usize] = BlockType::Stone;
                    } else if dy == 1 && dx == 2 && dz == 2 {
                        // Lava forge
                        chunk.blocks[x as usize][y][z as usize] = BlockType::Lava;
                    } else if dy == 1 && dx == -2 && dz == -2 {
                        // Chest
                        chunk.blocks[x as usize][y][z as usize] = BlockType::Chest;
                    } else if dy <= 3 && !is_edge {
                        chunk.blocks[x as usize][y][z as usize] = BlockType::Air;
                    }
                }
            }
        }
    }

    fn place_village_church(&self, chunk: &mut Chunk, local_x: usize, local_z: usize, ground_y: usize) {
        // 7x9 tall church with tower
        for dx in -3..=3 {
            for dz in -4..=4 {
                for dy in 0..=8 {
                    let x = local_x as i32 + dx;
                    let z = local_z as i32 + dz;
                    let y = ground_y + dy;

                    if x < 0 || x >= Self::CHUNK_SIZE as i32 ||
                       z < 0 || z >= Self::CHUNK_SIZE as i32 ||
                       y >= Self::CHUNK_HEIGHT {
                        continue;
                    }

                    let is_edge = dx.abs() == 3 || dz.abs() == 4;
                    let in_tower = dx.abs() <= 1 && dz >= 2;

                    if dy == 0 {
                        chunk.blocks[x as usize][y][z as usize] = BlockType::Cobblestone;
                    } else if dy <= 4 && is_edge {
                        // Main walls
                        chunk.blocks[x as usize][y][z as usize] = BlockType::Cobblestone;
                    } else if dy > 4 && dy <= 7 && in_tower && (dx.abs() == 1 || dz == 4) {
                        // Tower walls
                        chunk.blocks[x as usize][y][z as usize] = BlockType::Cobblestone;
                    } else if dy == 5 && !in_tower && dx.abs() <= 2 && dz.abs() <= 3 {
                        // Main roof
                        chunk.blocks[x as usize][y][z as usize] = BlockType::Stone;
                    } else if dy == 8 && in_tower && dx.abs() <= 1 && dz >= 2 && dz <= 4 {
                        // Tower roof
                        chunk.blocks[x as usize][y][z as usize] = BlockType::Stone;
                    } else if dy <= 4 && !is_edge {
                        chunk.blocks[x as usize][y][z as usize] = BlockType::Air;
                    }
                }
            }
        }
    }

    fn place_village_farm(&self, chunk: &mut Chunk, local_x: usize, local_z: usize, ground_y: usize) {
        // 9x9 fenced farm area with crops
        for dx in -4..=4 {
            for dz in -4..=4 {
                let x = local_x as i32 + dx;
                let z = local_z as i32 + dz;

                if x < 0 || x >= Self::CHUNK_SIZE as i32 ||
                   z < 0 || z >= Self::CHUNK_SIZE as i32 {
                    continue;
                }

                let is_edge = dx.abs() == 4 || dz.abs() == 4;

                // Ground level - farmland
                if ground_y < Self::CHUNK_HEIGHT {
                    if is_edge {
                        // Fence posts
                        chunk.blocks[x as usize][ground_y][z as usize] = BlockType::Dirt;
                        if ground_y + 1 < Self::CHUNK_HEIGHT {
                            chunk.blocks[x as usize][ground_y + 1][z as usize] = BlockType::Fence;
                        }
                    } else {
                        // Farmland with crops
                        chunk.blocks[x as usize][ground_y][z as usize] = BlockType::Dirt;
                        // Plant crops in alternating pattern
                        if ground_y + 1 < Self::CHUNK_HEIGHT && (dx + dz) % 2 == 0 {
                            chunk.blocks[x as usize][ground_y + 1][z as usize] = BlockType::Leaves;
                        }
                    }
                }

                // Water channel in center
                if dx == 0 && dz.abs() <= 2 && ground_y > 0 {
                    chunk.blocks[x as usize][ground_y][z as usize] = BlockType::Water;
                }
            }
        }
    }

    fn place_village_well(&self, chunk: &mut Chunk, local_x: usize, local_z: usize, ground_y: usize) {
        // 3x3 well with water and roof
        for dx in -1..=1 {
            for dz in -1..=1 {
                for dy in -2..=4 {
                    let x = local_x as i32 + dx;
                    let z = local_z as i32 + dz;
                    let y = ground_y as i32 + dy;

                    if x < 0 || x >= Self::CHUNK_SIZE as i32 ||
                       z < 0 || z >= Self::CHUNK_SIZE as i32 ||
                       y < 0 || y >= Self::CHUNK_HEIGHT as i32 {
                        continue;
                    }

                    let is_corner = dx.abs() == 1 && dz.abs() == 1;
                    let is_edge = dx.abs() == 1 || dz.abs() == 1;

                    if dy < 0 {
                        // Underground water
                        if !is_edge {
                            chunk.blocks[x as usize][y as usize][z as usize] = BlockType::Water;
                        } else {
                            chunk.blocks[x as usize][y as usize][z as usize] = BlockType::Cobblestone;
                        }
                    } else if dy == 0 {
                        // Well rim
                        if is_edge {
                            chunk.blocks[x as usize][y as usize][z as usize] = BlockType::Cobblestone;
                        } else {
                            chunk.blocks[x as usize][y as usize][z as usize] = BlockType::Water;
                        }
                    } else if dy <= 3 && is_corner {
                        // Support posts
                        chunk.blocks[x as usize][y as usize][z as usize] = BlockType::Fence;
                    } else if dy == 4 && is_edge {
                        // Roof
                        chunk.blocks[x as usize][y as usize][z as usize] = BlockType::Planks;
                    }
                }
            }
        }
    }

    fn place_village_paths(&self, chunk: &mut Chunk, local_x: usize, local_z: usize, ground_y: usize) {
        // Place gravel paths extending from structure
        for dx in -5..=5 {
            let x = local_x as i32 + dx;
            let z = local_z as i32;

            if x >= 0 && x < Self::CHUNK_SIZE as i32 && ground_y > 0 && ground_y < Self::CHUNK_HEIGHT {
                let current = chunk.blocks[x as usize][ground_y][z as usize];
                if current == BlockType::Grass || current == BlockType::Dirt {
                    chunk.blocks[x as usize][ground_y][z as usize] = BlockType::Gravel;
                }
            }
        }
        for dz in -5..=5 {
            let x = local_x as i32;
            let z = local_z as i32 + dz;

            if z >= 0 && z < Self::CHUNK_SIZE as i32 && ground_y > 0 && ground_y < Self::CHUNK_HEIGHT {
                let current = chunk.blocks[x as usize][ground_y][z as usize];
                if current == BlockType::Grass || current == BlockType::Dirt {
                    chunk.blocks[x as usize][ground_y][z as usize] = BlockType::Gravel;
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
            // Trigger water flow updates for adjacent water blocks
            self.trigger_water_updates_around(x, y, z);
            true
        } else {
            false
        }
    }

    pub fn place_torch(&mut self, x: i32, y: i32, z: i32, face: TorchFace) -> bool {
        if self.can_place_block_at(x, y, z) {
            self.set_block(x, y, z, BlockType::Torch);
            self.torch_orientations.insert((x, y, z), face);

            // Mark adjacent chunks as dirty since torch doesn't occlude faces
            // and neighboring blocks need their faces regenerated
            self.mark_neighbors_dirty(x, y, z);
            true
        } else {
            false
        }
    }

    fn mark_neighbors_dirty(&mut self, x: i32, _y: i32, z: i32) {
        // Mark all potentially affected chunks as dirty
        let offsets = [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)];
        for (dx, dz) in offsets {
            let chunk_x = (x + dx).div_euclid(Self::CHUNK_SIZE as i32);
            let chunk_z = (z + dz).div_euclid(Self::CHUNK_SIZE as i32);
            if let Some(chunk) = self.chunks.get_mut(&(chunk_x, chunk_z)) {
                chunk.dirty = true;
            }
        }
    }

    pub fn get_torch_face(&self, x: i32, y: i32, z: i32) -> Option<TorchFace> {
        self.torch_orientations.get(&(x, y, z)).copied()
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
            // Clean up torch orientation if it was a torch
            if block_type == BlockType::Torch {
                self.torch_orientations.remove(&pos);
            }
            // Trigger water flow updates for adjacent water blocks
            self.trigger_water_updates_around(x, y, z);
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

    /// Calculate water depth at a position (count of water blocks below)
    pub fn get_water_depth(&self, x: i32, y: i32, z: i32) -> i32 {
        (0..y)
            .rev()
            .take_while(|&check_y| self.get_block(x, check_y, z) == Some(BlockType::Water))
            .count() as i32
    }

    // ========== Water Flow System ==========

    /// Get water level at position (0 = no water, 8 = source, 1-7 = flowing)
    pub fn get_water_level(&self, x: i32, y: i32, z: i32) -> u8 {
        match self.get_block(x, y, z) {
            Some(BlockType::Water) => {
                // Check if we have a stored level, otherwise it's a source (8)
                *self.water_levels.get(&(x, y, z)).unwrap_or(&8)
            }
            _ => 0,
        }
    }

    /// Set water at position with given level. Level 0 removes water.
    fn set_water(&mut self, x: i32, y: i32, z: i32, level: u8) {
        if level == 0 {
            // Remove water
            if self.get_block(x, y, z) == Some(BlockType::Water) {
                self.set_block(x, y, z, BlockType::Air);
                self.water_levels.remove(&(x, y, z));
            }
        } else {
            // Set or update water
            let current_block = self.get_block(x, y, z);
            if current_block != Some(BlockType::Water) {
                self.set_block(x, y, z, BlockType::Water);
            }
            if level == 8 {
                // Source blocks don't need level stored (8 is default)
                self.water_levels.remove(&(x, y, z));
            } else {
                self.water_levels.insert((x, y, z), level);
            }
        }
    }

    /// Check if water can flow to this position
    fn can_water_flow_to(&self, x: i32, y: i32, z: i32) -> bool {
        if y < 0 || y >= Self::CHUNK_HEIGHT as i32 {
            return false;
        }
        match self.get_block(x, y, z) {
            Some(BlockType::Air) => true,
            Some(BlockType::Water) => true, // Can update existing water
            _ => false,
        }
    }

    /// Check if a block is solid (blocks water flow)
    fn is_solid_for_water(&self, x: i32, y: i32, z: i32) -> bool {
        match self.get_block(x, y, z) {
            Some(BlockType::Air) | Some(BlockType::Water) | None => false,
            Some(BlockType::Barrier) => true,
            _ => true,
        }
    }

    /// Queue a position for water update
    fn queue_water_update(&mut self, x: i32, y: i32, z: i32) {
        let pos = (x, y, z);
        // Avoid duplicate entries (simple check - not perfect but good enough)
        if !self.water_update_queue.iter().rev().take(100).any(|p| *p == pos) {
            self.water_update_queue.push_back(pos);
        }
    }

    /// Trigger water updates for blocks adjacent to a changed block
    pub fn trigger_water_updates_around(&mut self, x: i32, y: i32, z: i32) {
        // Check all 6 adjacent positions for water
        let offsets = [
            (1, 0, 0), (-1, 0, 0),
            (0, 1, 0), (0, -1, 0),
            (0, 0, 1), (0, 0, -1),
        ];
        for (dx, dy, dz) in offsets {
            let nx = x + dx;
            let ny = y + dy;
            let nz = z + dz;
            if self.get_block(nx, ny, nz) == Some(BlockType::Water) {
                self.queue_water_update(nx, ny, nz);
            }
        }
        // Also check the position itself if it became air (water above might flow down)
        if self.get_block(x, y, z) == Some(BlockType::Air) {
            // Check if there's water above that should flow down
            if self.get_block(x, y + 1, z) == Some(BlockType::Water) {
                self.queue_water_update(x, y + 1, z);
            }
        }
    }

    /// Process water updates using BFS algorithm
    /// Returns true if any updates were made
    pub fn process_water_updates(&mut self, max_updates: usize) -> bool {
        let mut updates_made = 0;
        let mut any_changes = false;

        while let Some((x, y, z)) = self.water_update_queue.pop_front() {
            if updates_made >= max_updates {
                // Put it back for next frame
                self.water_update_queue.push_front((x, y, z));
                break;
            }

            let level = self.get_water_level(x, y, z);
            if level == 0 {
                continue; // No water here anymore
            }

            updates_made += 1;

            // 1. Flow DOWN first (priority) - water flowing down becomes source-strength
            if y > 0 && self.can_water_flow_to(x, y - 1, z) {
                let below_level = self.get_water_level(x, y - 1, z);
                if below_level < 8 {
                    // Flow down with full strength
                    self.set_water(x, y - 1, z, 8);
                    self.queue_water_update(x, y - 1, z);
                    any_changes = true;
                }
            }

            // 2. Spread HORIZONTALLY if we have enough level
            if level > 1 {
                let new_level = level - 1;
                let horizontal_offsets = [(1, 0), (-1, 0), (0, 1), (0, -1)];

                for (dx, dz) in horizontal_offsets {
                    let nx = x + dx;
                    let nz = z + dz;

                    if self.can_water_flow_to(nx, y, nz) {
                        let neighbor_level = self.get_water_level(nx, y, nz);

                        // Only flow if we would increase the neighbor's level
                        if new_level > neighbor_level {
                            self.set_water(nx, y, nz, new_level);
                            self.queue_water_update(nx, y, nz);
                            any_changes = true;
                        }
                    }
                }
            }

            // 3. Check if this flowing water should disappear (no source feeding it)
            if level < 8 {
                // This is flowing water - check if it still has a valid source
                let has_source = self.check_water_source(x, y, z, level);
                if !has_source {
                    self.set_water(x, y, z, 0);
                    // Re-check neighbors
                    for (dx, dz) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                        let neighbor_level = self.get_water_level(x + dx, y, z + dz);
                        if neighbor_level > 0 && neighbor_level < 8 {
                            self.queue_water_update(x + dx, y, z + dz);
                        }
                    }
                    any_changes = true;
                }
            }
        }

        any_changes
    }

    /// Check if flowing water at position has a valid source feeding it
    fn check_water_source(&self, x: i32, y: i32, z: i32, current_level: u8) -> bool {
        // Check above - water above is always a valid source
        if self.get_water_level(x, y + 1, z) > 0 {
            return true;
        }

        // Check horizontal neighbors - need at least one with higher level
        for (dx, dz) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
            let neighbor_level = self.get_water_level(x + dx, y, z + dz);
            if neighbor_level > current_level {
                return true;
            }
        }

        false
    }

    // ========== End Water Flow System ==========

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