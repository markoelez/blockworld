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
- **world.rs** - Chunk-based voxel world with procedural generation using Perlin noise
  - `World` manages chunks (16x256x16 blocks each), loaded dynamically around player
  - `BlockType` enum defines all block types (Grass, Stone, Water, Torch, Chest, etc.)
  - `Biome` enum (Plains, Forest, Desert, Mountains, Tundra, Ocean) affects terrain generation
- **renderer.rs** - wgpu-based renderer (~4500 lines), handles all GPU operations
  - Greedy meshing for efficient chunk rendering
  - Separate opaque/transparent render passes
  - HDR rendering with bloom post-processing
  - Shadow mapping for sun shadows
- **camera.rs** - First-person camera with physics (gravity, collision detection, swimming)
- **entity.rs** - `EntityManager` handles `Villager` NPCs and `DroppedItem` pickups
- **particle.rs** - `ParticleSystem` for block break effects, weather (rain/snow), torch flames
- **ui.rs** - `Inventory`, `PauseMenu`, `DebugInfo` (F3), `ChestUI`, `UIRenderer`
- **audio.rs** - `AudioManager` using rodio for sound effects

### Shader Files (WGSL)

- **shader.wgsl** - Main block rendering with lighting, fog, foliage animation
- **water_shader.wgsl** - Gerstner wave animation, fresnel reflections, foam
- **cloud_shader.wgsl** - 3D clouds with day/night colors, drift animation
- **outline_shader.wgsl** - Targeted block highlight with pulsing glow
- **particle_shader.wgsl** - Billboard particle rendering
- **post_process_shader.wgsl** - Tone mapping, underwater effects
- **bloom_shader.wgsl** - Bloom extraction and blur passes

### Key Patterns

- Block types are passed to shaders as floats for texture atlas lookup
- `damage` vertex field: negative values (-1.0) flag special rendering (e.g., preview transparency)
- Chunks marked `dirty: true` trigger mesh regeneration in renderer
- Parallel mesh generation using rayon for chunk loading
