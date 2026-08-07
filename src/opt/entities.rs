//! Prefab entities scattered over the terrain.
//!
//! A third input image controls where a prefab goes. Each pixel of that image
//! is one TILE of the map. A black or fully transparent pixel puts no entity in
//! its tile. A white pixel always puts one there, and a value between the two
//! is the probability. The position in the tile is random, so a large white
//! area does not become a grid of trees.
//!
//! The entity map has its own size, which does not need to agree with the
//! heightmap. That size selects how large a tile is: a 64x64 entity map over a
//! 512x512 heightmap gives tiles of 8x8 terrain cells. This is how you ask for
//! one tree for each N cells.
//!
//! **Each entity gets its own brick grid.** The main grid holds the terrain,
//! and Brickadia does not let two bricks on one grid occupy the same space. A
//! tree must go a little into the ground, or it appears to float above a
//! sloped cell. A separate grid can be at any position and can go through the
//! main grid, so each entity becomes one `Entity_DynamicBrickGrid` with the
//! bricks of the prefab inside it. `--entity-sink` then controls how deep the
//! bricks of the prefab go into the ground.
//!
//! The prefab is a `.brz` file that you make in the game and give with
//! `--entity-prefab`. This module reads its bricks, puts the center of their
//! area at the origin, and puts their lowest face at Z zero. The bricks then
//! stand on the point that the placer selects.

use crate::map::*;
use crate::opt::{corners_at, rise_unit, sample_shared_vertices};
use crate::util::*;
use brdb::{
    Brick, BrickType, Brz, Entity, IntoReader, Position, Quat4f, Vector3f,
    assets::entities::{DYNAMIC_GRID, dynamic_grid_entity},
};
use log::{info, warn};

/// The largest number of entities that one render makes.
///
/// A white entity map at the size of the heightmap asks for one entity in each
/// cell. On a map of 512x512 with a tree of 539 bricks that is 141 million
/// bricks, which no computer can hold. The count comes from the entity map and
/// the density only, so the code can refuse before it reads the prefab or
/// makes one brick. The message tells the user which control to change.
pub const MAX_ENTITIES: usize = 50_000;

/// What to place, where, and how deep.
#[derive(Clone)]
pub struct EntityOptions {
    /// The bricks of the prefab, with the center of their area at the origin
    /// and their lowest face at Z zero. Use [`load_prefab`] to make them.
    pub prefab: Vec<Brick>,
    /// A number that multiplies the probability from each pixel. 1.0 keeps the
    /// value of the pixel, 0.5 gives half as many entities, and 0.0 gives
    /// none.
    pub density: f32,
    /// The units that each entity goes down into the ground. This hides the
    /// bottom face of the prefab and stops a tree from standing on one point
    /// of a sloped cell.
    pub sink: i32,
    /// If true, each entity gets its own angle around the vertical axis, and
    /// therefore its OWN grid. If false, every copy shares one grid, which is
    /// one entity in place of thousands.
    pub random_yaw: bool,
    /// The number that starts the random sequence. The same number always
    /// gives the same positions.
    pub seed: u64,
}

impl Default for EntityOptions {
    fn default() -> Self {
        Self {
            prefab: Vec::new(),
            density: 1.0,
            sink: 4,
            // Off by default: a turn of any size needs one grid for each copy,
            // and thousands of grids play badly.
            random_yaw: false,
            seed: 0,
        }
    }
}

/// Read the bricks of a prefab from the bytes of a `.brz` file.
///
/// The code moves the bricks so that the center of their area in X and Y is at
/// the origin and their lowest face is at Z zero. A prefab that you make in
/// the game keeps the position where you made it — `pine.brz` is at
/// `(678, 7691, 4)` — and without this correction each tree would go to that
/// same far position.
///
/// This takes bytes and not a path, because the browser version of the GUI has
/// no file system and gets the bytes from a dialog.
pub fn load_prefab(bytes: &[u8]) -> Result<Vec<Brick>, String> {
    let brz = Brz::read(&mut std::io::Cursor::new(bytes))
        .map_err(|e| format!("could not read the prefab: {e}"))?;
    let reader = brz.into_reader();
    let global = reader
        .global_data()
        .map_err(|e| format!("could not read the prefab: {e}"))?;

    let mut bricks = Vec::new();
    // Grid 0 does not exist in a prefab bundle, and a grid past the last one
    // gives an error rather than an empty list, so a missing grid ends the
    // loop instead of stopping the read.
    for grid in 0..4usize {
        let Ok(metas) = reader.brick_chunk_index(grid) else {
            continue;
        };
        for meta in metas {
            let soa = reader
                .brick_chunk_soa(grid, meta.index)
                .map_err(|e| format!("could not read the prefab: {e}"))?;
            for brick in soa.iter_bricks(meta.index, global.clone()) {
                bricks.push(brick.map_err(|e| format!("could not read the prefab: {e}"))?);
            }
        }
    }
    if bricks.is_empty() {
        return Err("the prefab has no bricks".to_string());
    }

    let (mut lo, mut hi) = ([i32::MAX; 3], [i32::MIN; 3]);
    for brick in &bricks {
        let size = match &brick.asset {
            BrickType::Procedural { size, .. } => [size.x as i32, size.y as i32, size.z as i32],
            // A brick from the catalog carries no size here. Its center still
            // bounds the prefab well enough to find the middle and the base.
            _ => [0; 3],
        };
        let p = [brick.position.x, brick.position.y, brick.position.z];
        for i in 0..3 {
            lo[i] = lo[i].min(p[i] - size[i]);
            hi[i] = hi[i].max(p[i] + size[i]);
        }
    }
    let offset = Position::new((lo[0] + hi[0]) / 2, (lo[1] + hi[1]) / 2, lo[2]);
    for brick in &mut bricks {
        brick.position -= offset;
        // The owner belongs to the SAVE and not to the brick. A prefab made in
        // the game names the people who built it -- `pine.brz` has a table of
        // two, and its bricks point at the second -- but the save this tool
        // writes has one owner. An index of 1 is then past the end of the
        // table, and the game refuses the whole bundle with "Invalid original
        // owner index 1 for brick 0" and gives no thumbnail. `None` lets the
        // write path use the owner of the save that receives the bricks.
        brick.owner_index = None;
        brick.original_owner_index = None;
        // The id links a brick to a wire or to a microchip. Each entity gets
        // its own copy of these bricks, so an id from the prefab would appear
        // thousands of times in one save.
        brick.id = None;
    }
    info!(
        "Prefab: {} brick(s), {}x{}x{} units",
        bricks.len(),
        hi[0] - lo[0],
        hi[1] - lo[1],
        hi[2] - lo[2],
    );
    Ok(bricks)
}

/// A small random sequence with a fixed result for a given seed and position.
///
/// The value comes from the coordinates and not from the sequence of the
/// visits, so the same map always gives the same forest. To go over the tiles
/// in a different sequence, or to skip a tile, does not move the other
/// entities.
fn hash(seed: u64, x: u32, y: u32, channel: u64) -> u64 {
    let mut v = seed
        ^ (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9)
        ^ channel.wrapping_mul(0x94D0_49BB_1331_11EB);
    v ^= v >> 30;
    v = v.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    v ^= v >> 27;
    v = v.wrapping_mul(0x94D0_49BB_1331_11EB);
    v ^ (v >> 31)
}

/// A value from 0.0 to 1.0.
fn unit(seed: u64, x: u32, y: u32, channel: u64) -> f32 {
    (hash(seed, x, y, channel) >> 40) as f32 / 16_777_216.0
}

/// The height of the surface of the terrain, in world units, for each cell.
///
/// Each surface mode puts its top face in a different place, so the placer
/// asks the mode rather than making its own guess. If it did not, a tree would
/// float above the ground in one mode and sink into it in another.
fn surface_heights(heightmap: &dyn Heightmap, options: &GenOptions) -> Vec<i32> {
    let (width, height) = heightmap.size();
    let count = width as usize * height as usize;
    let mut tops = vec![0i32; count];
    match options.surface {
        SurfaceMode::Terrain => {
            // The MEAN of the four corners, which is the height that the
            // middle of a sloped cell has. To use the lowest corner would put
            // a tree at the bottom of each slope.
            let (vertices, stride) = sample_shared_vertices(heightmap);
            let rise = rise_unit(options.scale);
            let floor = options.base_height() - 5;
            for y in 0..height {
                for x in 0..width {
                    let corners = corners_at(&vertices, stride, x, y);
                    let mean = corners.iter().map(|c| *c as i64).sum::<i64>() as f64 / 4.0;
                    tops[y as usize * width as usize + x as usize] =
                        floor + (mean * rise as f64).round() as i32;
                }
            }
        }
        SurfaceMode::Rampify => {
            // One plate for each `plates` of shade, and one more plate so that
            // a black pixel is still ground. This is the arithmetic in
            // `gen_rampify_heightmap`.
            let plates = ((options.scale as i32 + 2) / 4).max(1);
            let floor = options.base_height() - 5 - 4;
            for y in 0..height {
                for x in 0..width {
                    let shade = heightmap.at(x, y).min(i32::MAX as u32) as i32;
                    tops[y as usize * width as usize + x as usize] =
                        floor + (shade.saturating_mul(plates).saturating_add(1)) * 4;
                }
            }
        }
        SurfaceMode::Blocks => {
            // The top of the highest box, from `QuadTree::into_bricks`.
            let floor = options.base_height() - 5;
            for y in 0..height {
                for x in 0..width {
                    let shade = heightmap.at(x, y).min(i32::MAX as u32) as i32;
                    tops[y as usize * width as usize + x as usize] =
                        floor + shade.saturating_mul(options.scale as i32);
                }
            }
        }
    }
    tops
}

/// Put one grid of prefab bricks in each tile that the entity map selects.
///
/// The result goes to `World::add_brick_grid`, which moves the bricks to the
/// middle of their chunk. Each grid is separate from the main grid, so the
/// bricks can go into the terrain.
pub fn place_entities(
    heightmap: &dyn Heightmap,
    entity_map: &dyn Colormap,
    options: &GenOptions,
    entities: &EntityOptions,
) -> Result<Vec<(Entity, Vec<Brick>)>, String> {
    if entities.prefab.is_empty() {
        return Err("the entity prefab has no bricks".to_string());
    }
    let (width, height) = heightmap.size();
    let (tiles_x, tiles_y) = entity_map.size();
    if tiles_x == 0 || tiles_y == 0 {
        return Err("the entity map is empty".to_string());
    }
    if width == 0 || height == 0 {
        return Err("Heightmap is empty".to_string());
    }

    // Counted BEFORE the terrain heights are calculated and before one brick
    // is made. The count comes from the entity map and the density only, so a
    // request that cannot succeed fails in a moment and not after a minute.
    let mut wanted = 0usize;
    for ty in 0..tiles_y {
        for tx in 0..tiles_x {
            let pixel = entity_map.at(tx, ty);
            if chance(pixel, entities.density) > 0.0 {
                wanted += 1;
            }
        }
    }
    if wanted > MAX_ENTITIES {
        return Err(format!(
            "the entity map selects up to {wanted} entities, which is more than the limit of \
             {MAX_ENTITIES}. Each entity is a separate grid of {} brick(s). Use a smaller \
             entity map (its size gives the number of tiles), make it darker, or lower \
             --entity-density",
            entities.prefab.len(),
        ));
    }

    let tops = surface_heights(heightmap, options);
    let half = options.size as i32;
    let offset_x = -(width as i32 * half);
    let offset_y = -(height as i32 * half);

    // Where each copy goes, before the shape of the output is decided.
    let mut placements = Vec::new();
    for ty in 0..tiles_y {
        for tx in 0..tiles_x {
            let pixel = entity_map.at(tx, ty);
            let chance = chance(pixel, entities.density);
            if chance <= 0.0 || unit(entities.seed, tx, ty, 0) >= chance {
                continue;
            }

            // A point inside the area of the heightmap that this tile covers.
            // The tile is usually more than one cell wide, so the entity goes
            // to a random cell inside it and to a random point in that cell.
            let (x0, x1) = tile_span(tx, tiles_x, width);
            let (y0, y1) = tile_span(ty, tiles_y, height);
            let fx = x0 as f32 + unit(entities.seed, tx, ty, 1) * (x1 - x0) as f32;
            let fy = y0 as f32 + unit(entities.seed, tx, ty, 2) * (y1 - y0) as f32;
            let cell_x = (fx as u32).min(width - 1);
            let cell_y = (fy as u32).min(height - 1);

            // The cell is `half * 2` units wide and the terrain is centered on
            // the origin, so this is the same arithmetic that puts a cell of
            // the terrain in the world.
            placements.push(Vector3f {
                x: fx * (half * 2) as f32 + offset_x as f32,
                y: fy * (half * 2) as f32 + offset_y as f32,
                z: (tops[cell_y as usize * width as usize + cell_x as usize] - entities.sink)
                    as f32,
            });
        }
    }

    let grids = if entities.random_yaw {
        // A brick can turn by a quarter turn only, so an angle of any size has
        // to be the rotation of a GRID. Each copy therefore needs its own.
        //
        // The cost is one entity for each copy. The game holds each grid
        // separately, so a large wood becomes thousands of them and the frame
        // rate falls. `--entity-no-yaw` is the default for that reason.
        if placements.len() >= YAW_GRID_WARNING {
            warn!(
                "each of the {} entities with a random turn is a SEPARATE grid, because a \
                 brick can only turn by a quarter turn. The game holds each grid on its own, \
                 so this many will play badly. Remove --entity-yaw to put them all in one \
                 grid, or lower --entity-density",
                placements.len()
            );
        }
        placements
            .iter()
            .enumerate()
            .map(|(i, at)| {
                (
                    grid_entity(*at, unit(entities.seed, i as u32, 0, 3) * std::f32::consts::TAU),
                    entities.prefab.clone(),
                )
            })
            .collect()
    } else if placements.is_empty() {
        Vec::new()
    } else {
        // With no turn, a copy differs from the prefab by a MOVE alone, and a
        // move is what a brick position already expresses. Every copy can
        // therefore share one grid, which is one entity in place of thousands.
        //
        // The grid sits at the origin, so the position of a brick inside it is
        // the same world position that a brick of the terrain would use. It is
        // still a grid of its own, so its bricks may pass through the terrain.
        let mut bricks = Vec::with_capacity(placements.len() * entities.prefab.len());
        for at in &placements {
            // Rounded to whole units, because a brick position is an integer.
            // One unit is a tenth of a stud, so this moves nothing that anyone
            // can see.
            let offset = Position::new(
                at.x.round() as i32,
                at.y.round() as i32,
                at.z.round() as i32,
            );
            bricks.extend(entities.prefab.iter().map(|brick| {
                let mut brick = brick.clone();
                brick.position += offset;
                brick
            }));
        }
        vec![(grid_entity(Vector3f::default(), 0.0), bricks)]
    };

    info!(
        "Placed {} entit(ies) of {} brick(s) each: {} brick(s) over {} grid(s)",
        placements.len(),
        entities.prefab.len(),
        placements.len() * entities.prefab.len(),
        grids.len(),
    );
    if placements.is_empty() {
        warn!(
            "the entity map put no entities in the save: each of its pixels is black or fully \
             transparent, or --entity-density is 0"
        );
    }
    Ok(grids)
}

/// The number of turned entities above which the render says that the save
/// will play badly.
///
/// Each one is a separate grid, and the game holds each grid on its own.
const YAW_GRID_WARNING: usize = 256;

/// One brick grid, at a world position and turned by `yaw` radians.
fn grid_entity(location: Vector3f, yaw: f32) -> Entity {
    Entity {
        asset: DYNAMIC_GRID,
        location,
        rotation: yaw_quat(yaw),
        // The grid must not fall or move: it is scenery, and a tree that rolls
        // down a hill when the map loads is not scenery.
        frozen: true,
        sleeping: true,
        data: dynamic_grid_entity(),
        ..Default::default()
    }
}

/// The probability that one pixel of the entity map gives.
///
/// Black gives 0.0 and white gives 1.0, so a value between them is a
/// probability. A fully transparent pixel gives 0.0, whatever its color.
fn chance(pixel: [u8; 4], density: f32) -> f32 {
    if pixel[3] == 0 {
        return 0.0;
    }
    // The mean of the three color channels, so a colored entity map still
    // works. A user who paints in green expects the green to count.
    let luma = (pixel[0] as f32 + pixel[1] as f32 + pixel[2] as f32) / (3.0 * 255.0);
    (luma * density).clamp(0.0, 1.0)
}

/// The first cell and the last cell of the heightmap that tile `index` covers.
///
/// The entity map can be smaller than the heightmap, larger than it, or the
/// same size. This divides the cells over the tiles without a gap and without
/// an overlap.
fn tile_span(index: u32, tiles: u32, cells: u32) -> (u32, u32) {
    let low = (index as u64 * cells as u64 / tiles as u64) as u32;
    let high = (((index as u64 + 1) * cells as u64 / tiles as u64) as u32).max(low + 1);
    (low, high.min(cells))
}

/// A rotation of `yaw` radians around the vertical axis, as a quaternion.
fn yaw_quat(yaw: f32) -> Quat4f {
    Quat4f {
        x: 0.0,
        y: 0.0,
        z: (yaw / 2.0).sin(),
        w: (yaw / 2.0).cos(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Flat(u32, u32, u32);
    impl Heightmap for Flat {
        fn at(&self, _x: u32, _y: u32) -> u32 {
            self.2
        }
        fn size(&self) -> (u32, u32) {
            (self.0, self.1)
        }
    }
    /// An entity map of one value.
    struct Solid(u32, u32, [u8; 4]);
    impl Colormap for Solid {
        fn at(&self, _x: u32, _y: u32) -> [u8; 4] {
            self.2
        }
        fn size(&self) -> (u32, u32) {
            (self.0, self.1)
        }
    }

    fn options() -> GenOptions {
        GenOptions {
            size: 5,
            scale: 4,
            asset: brdb::assets::bricks::PB_DEFAULT_MICRO_BRICK,
            cull: false,
            micro: false,
            stud: false,
            snap: false,
            img: false,
            glow: false,
            hdmap: false,
            lrgb: false,
            nocollide: false,
            quadtree: true,
            greedy: false,
            surface: SurfaceMode::Terrain,
        }
    }

    fn one_brick() -> Vec<Brick> {
        vec![Brick::default()]
    }

    /// The number of copies, whatever shape the result took: one grid holding
    /// every copy, or one grid for each of them.
    fn copies(grids: &[(Entity, Vec<Brick>)], prefab_len: usize) -> usize {
        grids.iter().map(|(_, b)| b.len()).sum::<usize>() / prefab_len.max(1)
    }

    /// The position of each copy in the one shared grid. The tests use a
    /// prefab of one brick, so a brick is a copy.
    fn shared_positions(grids: &[(Entity, Vec<Brick>)]) -> Vec<(i32, i32)> {
        assert_eq!(grids.len(), 1, "this reads the shape with one shared grid");
        grids[0]
            .1
            .iter()
            .map(|b| (b.position.x, b.position.y))
            .collect()
    }

    fn entity_options() -> EntityOptions {
        EntityOptions {
            prefab: one_brick(),
            ..Default::default()
        }
    }

    /// The owner belongs to the save, not to the brick.
    ///
    /// A prefab made in the game names the people who built it. `pine.brz`
    /// carries a table of two owners and its bricks point at the second one.
    /// The save that this tool writes has ONE owner, so an index of 1 is past
    /// the end of its table: the game refuses the full bundle with "Invalid
    /// original owner index 1 for brick 0" and shows no thumbnail. Nothing in
    /// the types can hold this, and a save that the encoder accepts can still
    /// fail in the game, so the test goes through a real `.brz`.
    #[test]
    fn a_prefab_forgets_the_owners_of_the_save_it_came_from() {
        let mut world = brdb::World::new();
        world.add_bricks(vec![
            Brick {
                position: Position::new(10, 20, 30),
                owner_index: Some(0),
                original_owner_index: Some(0),
                id: Some(7),
                ..Default::default()
            },
            Brick {
                position: Position::new(-10, -20, 8),
                owner_index: Some(0),
                original_owner_index: Some(0),
                ..Default::default()
            },
        ]);
        let bytes = world.to_brz_vec().expect("the fixture must encode");

        let bricks = load_prefab(&bytes).expect("the fixture must read back");
        assert_eq!(bricks.len(), 2);
        for brick in &bricks {
            assert_eq!(
                (brick.owner_index, brick.original_owner_index, brick.id),
                (None, None, None),
                "a brick of a prefab must point at no owner and no id of its own save"
            );
        }
    }

    /// The bricks must move to the origin, or each entity would go to the
    /// position at which the prefab was saved. `pine.brz` was made at
    /// `(678, 7691, 4)`.
    #[test]
    fn a_prefab_moves_to_the_origin_and_stands_on_zero() {
        let mut world = brdb::World::new();
        world.add_bricks(vec![
            Brick {
                position: Position::new(700, 7700, 100),
                ..Default::default()
            },
            Brick {
                position: Position::new(660, 7660, 20),
                ..Default::default()
            },
        ]);
        let bytes = world.to_brz_vec().unwrap();
        let bricks = load_prefab(&bytes).unwrap();

        // The two bricks are the same size, so the middle of the pair goes to
        // x 0 and y 0 and the lower one stands at z 0.
        let (mut lo, mut hi) = ([i32::MAX; 3], [i32::MIN; 3]);
        for brick in &bricks {
            let size = match &brick.asset {
                BrickType::Procedural { size, .. } => {
                    [size.x as i32, size.y as i32, size.z as i32]
                }
                _ => [0; 3],
            };
            let p = [brick.position.x, brick.position.y, brick.position.z];
            for i in 0..3 {
                lo[i] = lo[i].min(p[i] - size[i]);
                hi[i] = hi[i].max(p[i] + size[i]);
            }
        }
        assert_eq!(lo[0] + hi[0], 0, "the prefab must be centered on x 0");
        assert_eq!(lo[1] + hi[1], 0, "the prefab must be centered on y 0");
        assert_eq!(lo[2], 0, "the prefab must stand on z 0");
    }

    /// Black puts no entity and white puts one in each tile. This is the rule
    /// that the entity map states.
    #[test]
    fn black_places_nothing_and_white_places_one_entity_in_each_tile() {
        let black = place_entities(
            &Flat(32, 32, 4),
            &Solid(8, 8, [0, 0, 0, 255]),
            &options(),
            &entity_options(),
        )
        .unwrap();
        assert!(black.is_empty(), "black must place no entity");

        let white = place_entities(
            &Flat(32, 32, 4),
            &Solid(8, 8, [255, 255, 255, 255]),
            &options(),
            &entity_options(),
        )
        .unwrap();
        assert_eq!(copies(&white, 1), 64, "white must place one entity in each tile");
    }

    /// A transparent pixel places nothing, whatever its color. A user who
    /// paints on a transparent background must not get a full forest.
    #[test]
    fn a_transparent_pixel_places_nothing_however_bright_it_is() {
        let placed = place_entities(
            &Flat(16, 16, 4),
            &Solid(4, 4, [255, 255, 255, 0]),
            &options(),
            &entity_options(),
        )
        .unwrap();
        assert!(placed.is_empty());
    }

    /// The entity map gives the number of tiles, so it can be smaller than the
    /// heightmap. This is how a user asks for one entity for each N cells.
    #[test]
    fn the_entity_map_size_gives_the_number_of_tiles() {
        for tiles in [2u32, 8, 16] {
            let placed = place_entities(
                &Flat(64, 64, 4),
                &Solid(tiles, tiles, [255, 255, 255, 255]),
                &options(),
                &entity_options(),
            )
            .unwrap();
            assert_eq!(copies(&placed, 1) as u32, tiles * tiles);
        }
    }

    /// Each entity must be inside the area of the terrain. A tree outside it
    /// stands on nothing.
    #[test]
    fn every_entity_is_inside_the_area_of_the_terrain() {
        let (cells, half) = (32i32, 5i32);
        let placed = place_entities(
            &Flat(32, 32, 4),
            &Solid(16, 16, [255, 255, 255, 255]),
            &options(),
            &entity_options(),
        )
        .unwrap();
        let edge = cells * half;
        for (x, y) in shared_positions(&placed) {
            assert!(
                x >= -edge && x <= edge,
                "x {x} is outside -{edge}..{edge}"
            );
            assert!(
                y >= -edge && y <= edge,
                "y {y} is outside -{edge}..{edge}"
            );
        }
    }

    /// The positions come from the seed and the coordinates, so one seed
    /// always gives the same forest and a different seed gives a different
    /// one.
    #[test]
    fn the_seed_decides_the_positions_and_repeats_them_exactly() {
        let place = |seed| {
            shared_positions(
                &place_entities(
                    &Flat(64, 64, 4),
                    &Solid(8, 8, [128, 128, 128, 255]),
                    &options(),
                    &EntityOptions {
                        seed,
                        ..entity_options()
                    },
                )
                .unwrap(),
            )
        };
        assert_eq!(place(7), place(7), "one seed must repeat exactly");
        assert_ne!(place(7), place(8), "a different seed must move the entities");
    }

    /// With no turn, every copy shares ONE grid. A copy then differs from the
    /// prefab by a move alone, which a brick position already expresses, so
    /// thousands of separate grids would cost the game a great deal for
    /// nothing. This is the default.
    #[test]
    fn without_a_turn_every_copy_shares_one_grid() {
        let placed = place_entities(
            &Flat(64, 64, 4),
            &Solid(8, 8, [255, 255, 255, 255]),
            &options(),
            &entity_options(),
        )
        .unwrap();
        assert!(
            !EntityOptions::default().random_yaw,
            "one shared grid must be the default"
        );
        assert_eq!(placed.len(), 1, "every copy must share one grid");
        assert_eq!(placed[0].1.len(), 64, "the one grid must hold every copy");

        // The copies must actually be at different places: one grid is of no
        // use if each copy sits on top of the last.
        let mut seen = shared_positions(&placed);
        seen.sort_unstable();
        seen.dedup();
        assert!(seen.len() > 32, "the copies collapsed to {} places", seen.len());
    }

    /// A brick can turn by a quarter turn only, so an angle of any size has to
    /// be the rotation of a GRID. Each copy then needs its own, and each gets
    /// its own angle.
    #[test]
    fn a_random_turn_gives_each_copy_its_own_grid_and_its_own_angle() {
        let placed = place_entities(
            &Flat(64, 64, 4),
            &Solid(8, 8, [255, 255, 255, 255]),
            &options(),
            &EntityOptions {
                random_yaw: true,
                ..entity_options()
            },
        )
        .unwrap();
        assert_eq!(placed.len(), 64, "each copy needs a grid of its own");
        assert!(placed.iter().all(|(_, bricks)| bricks.len() == 1));

        let mut angles: Vec<i32> = placed
            .iter()
            // The quaternion is a turn around the vertical axis only, so its z
            // holds the angle.
            .map(|(e, _)| (e.rotation.z * 1000.0) as i32)
            .collect();
        angles.sort_unstable();
        angles.dedup();
        assert!(angles.len() > 32, "the copies share {} angle(s)", angles.len());
    }

    /// A grey pixel is a probability, so it must place more entities than a
    /// dark pixel and fewer than white.
    #[test]
    fn a_grey_pixel_places_some_entities_but_not_all_of_them() {
        let count = |v: u8| {
            let placed = place_entities(
                &Flat(64, 64, 4),
                &Solid(24, 24, [v, v, v, 255]),
                &options(),
                &entity_options(),
            )
            .unwrap();
            copies(&placed, 1)
        };
        let (dark, grey, white) = (count(40), count(128), count(255));
        assert_eq!(white, 24 * 24);
        assert!(
            dark < grey && grey < white,
            "the counts must increase with the value of the pixel: {dark}, {grey}, {white}"
        );
    }

    /// `--entity-density` must scale the probability, so a user can thin a
    /// forest without painting the map again.
    #[test]
    fn density_scales_the_number_of_entities() {
        let count = |density| {
            let placed = place_entities(
                &Flat(64, 64, 4),
                &Solid(24, 24, [255, 255, 255, 255]),
                &options(),
                &EntityOptions {
                    density,
                    ..entity_options()
                },
            )
            .unwrap();
            copies(&placed, 1)
        };
        assert_eq!(count(1.0), 24 * 24);
        assert_eq!(count(0.0), 0);
        let half = count(0.5);
        assert!(
            (200..=400).contains(&half),
            "a density of 0.5 gave {half} of 576 entities"
        );
    }

    /// An entity stands ON the surface, less the depth that it goes into it.
    /// The height must follow the terrain and must not stay at zero.
    #[test]
    fn an_entity_stands_on_the_surface_of_its_own_cell() {
        let opts = options();
        for shade in [0u32, 10, 60] {
            let placed = place_entities(
                &Flat(16, 16, shade),
                &Solid(4, 4, [255, 255, 255, 255]),
                &opts,
                &EntityOptions {
                    sink: 4,
                    ..entity_options()
                },
            )
            .unwrap();
            // A flat map has one height, so each copy must be at it. The one
            // shared grid sits at the origin, so the Z of a brick inside it is
            // the world height.
            let expect =
                opts.base_height() - 5 + shade as i32 * rise_unit(opts.scale) - 4;
            for brick in &placed[0].1 {
                assert_eq!(
                    brick.position.z, expect,
                    "a copy is at z {} but the surface is at {expect}",
                    brick.position.z
                );
            }
        }
    }

    /// The refusal must come from the entity map alone, so it arrives before
    /// the render rather than after it.
    #[test]
    fn an_entity_map_that_asks_for_too_many_entities_is_refused_by_name() {
        let side = (MAX_ENTITIES as f64).sqrt() as u32 + 8;
        let Err(err) = place_entities(
            &Flat(64, 64, 4),
            &Solid(side, side, [255, 255, 255, 255]),
            &options(),
            &entity_options(),
        ) else {
            panic!("an entity map past the limit must be refused");
        };
        assert!(
            err.contains("--entity-density") && err.contains(&MAX_ENTITIES.to_string()),
            "the message must name the limit and a control to change: {err}"
        );
    }

    /// The tiles must cover the cells, whether the entity map is smaller than
    /// the heightmap, larger than it, or the same size.
    ///
    /// With fewer tiles than cells, each tile takes its own group of cells:
    /// the groups follow one another with no gap and no overlap. With MORE
    /// tiles than cells, several tiles share a cell, which is how a user asks
    /// for more than one entity in a cell. In each case a tile must hold at
    /// least one cell and must stay inside the map, or an entity would have no
    /// ground under it.
    #[test]
    fn the_tiles_cover_the_cells_at_each_relative_size() {
        for (tiles, cells) in [(8u32, 64u32), (64, 8), (7, 7), (3, 10), (10, 3)] {
            let mut previous = 0;
            for index in 0..tiles {
                let (low, high) = tile_span(index, tiles, cells);
                assert!(low < high, "the tile {index} of {tiles} holds no cell");
                assert!(high <= cells, "the tile {index} of {tiles} goes past the map");
                if tiles <= cells {
                    assert_eq!(
                        low, previous,
                        "with {tiles} tiles over {cells} cells, the tile {index} does not \
                         follow the tile before it"
                    );
                    previous = high;
                }
            }
            if tiles <= cells {
                assert_eq!(previous, cells, "the tiles must reach the end of the map");
            }
        }
    }
}
