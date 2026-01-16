# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build Commands

```bash
cargo build --release    # Build optimized binary
cargo run --release      # Build and run the game
```

No tests are currently configured for this project.

## Architecture Overview

BlockWorld is a Minecraft-inspired voxel game built with Rust and wgpu. The rendering uses an HDR pipeline with post-processing effects.

### Core Modules

- **main.rs** - Game loop using winit event loop, handles input and coordinates all systems
  - Key bindings: WASD movement, E place/eat, R break/attack/respawn
  - Integrates survival mechanics, combat, and entity updates

- **world.rs** - Chunk-based voxel world with procedural generation
  - `World` manages chunks (16x256x16 blocks each), loaded dynamically around player
  - `BlockType` enum defines all block types including food items (RawPork, RawBeef, etc.)
  - `Biome` enum (Plains, Forest, Desert, Mountains, Tundra, Ocean) affects terrain generation
  - Generates caves, ore veins, dungeons, mineshafts, and villages
  - `BlockType::food_properties()` returns hunger/saturation restore values for food items

- **renderer.rs** - wgpu-based renderer (~5000+ lines), handles all GPU operations
  - Greedy meshing for efficient chunk rendering
  - Separate opaque/transparent render passes
  - HDR rendering with bloom post-processing
  - Shadow mapping for sun shadows
  - Entity rendering: villagers, animals, hostile mobs, dropped items
  - `update_hostile_mob_mesh()` renders zombies with walking/attack animations

- **camera.rs** - First-person camera with physics and survival mechanics
  - Physics: gravity, collision detection, swimming, fall tracking
  - Survival stats: `health`, `hunger`, `saturation`, `air_supply`, `damage_cooldown`
  - Methods: `take_damage()`, `heal()`, `eat_food()`, `respawn()`, `update_survival()`
  - `HungerAction` enum for different hunger depletion rates
  - Fall damage calculated on landing (>3 blocks)
  - Lava and drowning damage in `update_survival()`

- **entity.rs** - Entity management system (~2200 lines)
  - `EntityManager` handles all entity types
  - `Villager` - NPCs with wandering AI
  - `Animal` - 14 animal types with health, AI states, physics
    - `AnimalType` enum with movement types (Ground, Aquatic, Flying)
    - `meat_drop()` returns food type and quantity for each animal
  - `HostileMob` - Zombies with chase/attack AI
    - States: Idle, Wandering, Chasing, Attacking
    - Spawns 24-128 blocks from player, max 30 active
  - `DroppedItem` - Block pickups with bobbing animation

- **particle.rs** - Particle effects
  - `ParticleSystem` for block break, weather (rain/snow), torch flames
  - `LightningBolt` for storm effects
  - `WeatherState` manages weather transitions

- **ui.rs** - User interface rendering
  - `Inventory` with 6 hotbar slots
  - `PauseMenu`, `DebugInfo` (F3), `ChestUI`
  - `render_survival_ui()` - Health hearts, hunger drumsticks, air bubbles
  - `render_death_screen()` - Death overlay with respawn prompt
  - `block_type_to_ui_index()` maps BlockType to texture atlas index

- **audio.rs** - Sound system using rodio
  - `AudioManager` for sound effects (footsteps, block sounds, splash, thunder)
  - `MusicManager` for procedural ambient music
  - Volume levels are intentionally low (~0.01-0.15)

### Shader Files (WGSL)

- **shader.wgsl** - Main block rendering with lighting, fog, foliage animation
- **water_shader.wgsl** - Gerstner wave animation, fresnel reflections, foam
- **cloud_shader.wgsl** - 3D clouds with day/night colors, drift animation
- **sky_shader.wgsl** - Atmospheric scattering, sun/moon, stars
- **outline_shader.wgsl** - Targeted block highlight with pulsing glow
- **particle_shader.wgsl** - Billboard particle rendering
- **post_process_shader.wgsl** - Tone mapping, underwater effects
- **bloom_shader.wgsl** - Bloom extraction and blur passes

### Key Patterns

- Block types are passed to shaders as floats for texture atlas lookup
- `damage` vertex field: negative values (-1.0) flag special rendering (e.g., preview transparency)
- Chunks marked `dirty: true` trigger mesh regeneration in renderer
- Parallel mesh generation using rayon for chunk loading
- Entity buffers are pre-allocated with max capacities (200 animals, 30 hostile mobs)

### Survival System Details

**Health & Damage:**
- 20 HP (10 hearts), 0.5s damage cooldown
- Fall damage: `(fall_distance - 3).floor()` for falls > 3 blocks
- Lava: 4 damage/sec, Drowning: 2 damage/sec when air depleted

**Hunger:**
- 20 hunger (10 drumsticks), saturation buffer consumed first
- Depletion: Walk 0.01, Jump 0.05, Sprint 0.03, Attack 0.1
- At 0 hunger: 1 starvation damage every 4 seconds
- At 18+ hunger: heal 1 HP every 0.5s (costs 1.5 hunger)
- Below 6 hunger: cannot sprint

**Combat:**
- R key attacks closest mob/animal within 4 blocks
- 1 damage per hit (fist), knockback applied
- Animals flee when damaged, drop meat on death
- Hostile mobs attack when within 2 blocks

**Food Items:**
- RawPork, RawBeef: +3 hunger, +1.8 saturation
- RawChicken, RawMutton: +2 hunger, +1.2 saturation
- E key to eat when holding food (can't eat if hunger full)

### Adding New Features

**New Block Types:**
1. Add variant to `BlockType` enum in `world.rs`
2. Add to `block_type_to_ui_index()` in `ui.rs` for hotbar display
3. If food: add to `food_properties()` in `world.rs`

**New Hostile Mob:**
1. Add variant to `HostileMobType` enum in `entity.rs`
2. Implement stats methods (health, damage, speed, detection_range)
3. Update `update_hostile_mob_mesh()` in `renderer.rs` for rendering

**New Animal:**
1. Add variant to `AnimalType` enum in `entity.rs`
2. Implement required methods (dimensions, speed, movement_type, etc.)
3. Add to `meat_drop()` if it should drop food
4. Update biome spawn logic in `try_spawn_animals()`
