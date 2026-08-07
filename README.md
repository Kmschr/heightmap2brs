# Heightmap2BRZ

This tool functions as an img2brz, img2text, and heightmap2brz

[Download here](https://github.com/Meshiest/heightmap2brz/releases)

![Example output](https://i.imgur.com/QdPLN09.png)
![GTAV Map](https://i.imgur.com/J9XpmT3.png)
![Gui](https://i.imgur.com/8v9MXnl.png)

### Compiling

You need [rust](https://www.rust-lang.org/).

Run `cargo build` for the CLI, `cargo build --bin heightmap_gui --features gui` for the gui.

### Usage

Compile or download from releases.

`heightmap.exe --help` for usage instructions:

```
USAGE:
    heightmap.exe [FLAGS] [OPTIONS] <INPUT>...

FLAGS:
        --cull         Automatically remove bottom level bricks and fully transparent bricks
        --entities <img>  Scatter --entity-prefab over the terrain from a third image
        --glow         Make the heightmap glow at 0 intensity
        --greedy       Use greedy optimization
    -h, --help         Prints help information
        --hdmap        Using a high detail rgb color encoded heightmap
    -i, --img          Make the heightmap flat and render an image
        --lrgb         Use linear rgb input color instead of sRGB
        --micro        Render bricks as micro bricks
        --nocollide    Disable brick collision
        --prefab       Write a prefab bundle instead of a world
        --rampify      Rampify the terrain with Wrapperup's rampifier
        --smooth       Render bricks as smooth tiles
        --snap         Snap bricks to the brick grid
        --stud         Render bricks as stud cubes
        --terrain      Render the terrain as smooth micro bricks
        --text         Render the input image as TextDisplay component bricks
        --tile         Render bricks as tiles
    -V, --version      Prints version information

OPTIONS:
        --alpha-threshold <alphathreshold>    Text mode: alpha below this is transparent (default 128)
        --char-repeat <charrepeat>            Text mode: glyphs emitted per pixel (default 2)
    -c, --colormap <colormap>                 Input colormap image (PNG/JPG)
        --empty-char <emptychar>              Text mode: glyph for transparent pixels (default space)
        --fill-char <fillchar>                Text mode: glyph for opaque pixels (default █)
        --font <font>                         Text mode: font preset (monaspace, iosevka, orbitron; default monaspace)
        --line-height-world <lineheight>      Text mode: world units per pixel row / pixel size (default 1)
    -o, --output <output>                     Output file (BRDB, BRZ)
    -s, --size <size>                         Brick stud size (default 1)
    -v, --vertical <vertical>                 Vertical scale multiplier (default 1)

ARGS:
    <INPUT>...    Input heightmap image files (PNG/JPG)
```

### Smooth surfaces

By default every pixel becomes a flat-topped prism, so a slope renders as a
staircase. Two flags replace that with a sloped surface. They are alternatives
to each other and to `--tile`/`--smooth`/`--micro`/`--stud`, and they choose
their own bricks per cell, so the optimizer flags (`--greedy`, `--snap`) do not
apply to them.

`--terrain` builds the surface out of **micro wedges**. Heights are sampled on a
shared `(w+1) x (h+1)` vertex grid — each vertex is the mean of the pixels
touching it — and every cell picks the assembly from Brickadia's micro wedge
family (ramp, wedge corner, inner corner, or a stacked outer corner plus
triangle) that best matches its four corner heights. Neighbouring cells quote
the same vertices, so their surfaces meet rather than step. Every candidate is
fitted *from below*, so no cell can protrude through its neighbour and a clean
cliff comes out as one exact steep ramp instead of a shelf. Foundations are a
flat-topped field, so they are greedy-merged into boxes rather than left one per
pixel. Costs roughly 0.7 to 2.5 bricks per pixel, depending on how much of the
map is flat and uniformly coloured. Under `--terrain`, `--vertical` is the height of one
shade of grey in units, rounded up to an even number (a wedge's half height has
to be a whole unit).

`heightmap heightmap.png -c colormap.png --terrain -v 4 -o terrain.brz`

`--rampify` runs [Wrapperup's rampifier](https://github.com/Wrapperup/rampifier)
over the height columns instead: it fits full-size `PB_DefaultRamp`,
`PB_DefaultWedge` and ramp corner pieces onto the surface and fills the rest
with plain bricks. Coarser than `--terrain` — one plate of vertical resolution,
runs of at most 4 studs — but it builds from ordinary bricks rather than micro
pieces, and flat ground merges into large blocks. Under `--rampify`,
`--vertical` is rounded to a whole number of plates (4 units).

`heightmap heightmap.png -c colormap.png --rampify -v 8 -o rampified.brz`

### Scattering entities

`--entities` takes a **third image** and scatters a prefab over the terrain.
Each of its pixels is one *tile* of the map: black or fully transparent places
nothing, white always places one entity somewhere in that tile, and a value
between the two is the probability. The position inside the tile is random, so
a large white area does not come out as a grid.

The image has its own size, and that size chooses how big a tile is — a 96x96
entity map over a 384x384 heightmap gives one tile per 4x4 terrain cells. That
is how you ask for one tree per N cells.

`--entity-prefab` is the `.brz` to place. Save one in the game, then:

`heightmap forest_hm.png -c forest_cm.png --terrain --size 8 -v 12 --entities forest_em.png --entity-prefab pine.brz -o forest.brz`

**Each entity becomes its own brick grid.** Two bricks on one grid cannot
occupy the same space, so a tree on the main grid would have to sit exactly on
the surface and would appear to float over a sloped cell. A separate grid can
be at any position and can pass through the main grid, so `--entity-sink`
(default 4 units) simply pushes each prefab that far into the ground.

The rest: `--entity-density` multiplies every pixel's probability (thin a
forest without repainting the map), `--entity-seed` picks the layout (the same
number always gives the same forest), and `--entity-no-yaw` turns off the
random rotation that stops a forest of one prefab looking like copies.

The GUI has the same controls in an **Entities** row.

Add `--prefab` to either (or to any heightmap/`--img` render) to write a prefab
bundle instead of a world, so the save can be dropped into Brickadia's `Prefabs`
folder and spawned from the prefab browser.

In the GUI both appear in the **Brick Type** row as *Smooth Terrain* and
*Rampify*.

###  Examples

An example command for generating the GTA V map would be:

`heightmap example_maps/gta5_fixed2_height.png -c example_maps/gta5_fixed2_color.png -s 4 -v 20 --tile -o gta5.brz`

To use stacked heightmap for increased resolution, simply provide more input files. See the `stacked_N.png` files in the `example_maps` directory for example stacked heightmaps.

`heightmap ./example_maps/stacked_1.png ./example_maps/stacked_2.png ./example_maps/stacked_3.png ./example_maps/stacked_4.png --tile`

To generate HD heightmaps for the `--hdmap` flag, check out [Kmschr's GeoTIFF2Heightmap tool](https://github.com/Kmschr/GeoTIFF2Heightmap).
