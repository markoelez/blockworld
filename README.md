# BlockWorld - 3D Minecraft Clone

A basic 3D voxel world implementation in Rust using wgpu for rendering.

## Features
- 3D terrain generation using Perlin noise
- Basic player movement (WASD + Space/Shift)
- Block rendering with simple lighting
- Chunk-based world system

## Running
```bash
cargo run
```

## Controls
- W/A/S/D - Move forward/left/backward/right
- Space - Move up
- Left Shift - Move down
- ESC - Quit

## Implementation Details
- Uses wgpu for GPU rendering
- Perlin noise for terrain generation
- Chunk size: 16x16x64 blocks
- 3x3 chunks generated around origin