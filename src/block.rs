/// All block types in the game. The discriminant doubles as the save-file id.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[repr(u8)]
pub enum Block {
    Air = 0,
    Grass,
    Dirt,
    Stone,
    Cobblestone,
    Sand,
    Water,
    Wood,
    Leaves,
    Planks,
    CoalOre,
    IronOre,
    Bedrock,
    Torch,
    StoneBrick,
}

pub const ALL_BLOCKS: [Block; 15] = [
    Block::Air,
    Block::Grass,
    Block::Dirt,
    Block::Stone,
    Block::Cobblestone,
    Block::Sand,
    Block::Water,
    Block::Wood,
    Block::Leaves,
    Block::Planks,
    Block::CoalOre,
    Block::IronOre,
    Block::Bedrock,
    Block::Torch,
    Block::StoneBrick,
];

pub type Rgb = (u8, u8, u8);

impl Block {
    pub fn to_u8(self) -> u8 {
        self as u8
    }

    pub fn from_u8(v: u8) -> Block {
        ALL_BLOCKS.get(v as usize).copied().unwrap_or(Block::Air)
    }

    /// Solid blocks stop entities.
    pub fn is_solid(self) -> bool {
        !matches!(self, Block::Air | Block::Water | Block::Torch)
    }

    pub fn is_minable(self) -> bool {
        !matches!(self, Block::Air | Block::Water | Block::Bedrock)
    }

    /// What lands in your inventory when you mine this block.
    pub fn drops(self) -> Option<Block> {
        match self {
            Block::Air | Block::Water | Block::Bedrock => None,
            Block::Grass => Some(Block::Dirt),
            Block::Stone => Some(Block::Cobblestone),
            b => Some(b),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Block::Air => "Air",
            Block::Grass => "Grass",
            Block::Dirt => "Dirt",
            Block::Stone => "Stone",
            Block::Cobblestone => "Cobblestone",
            Block::Sand => "Sand",
            Block::Water => "Water",
            Block::Wood => "Wood",
            Block::Leaves => "Leaves",
            Block::Planks => "Planks",
            Block::CoalOre => "Coal Ore",
            Block::IronOre => "Iron Ore",
            Block::Bedrock => "Bedrock",
            Block::Torch => "Torch",
            Block::StoneBrick => "Stone Brick",
        }
    }

    pub fn glyph(self) -> char {
        match self {
            Block::Air => ' ',
            Block::Grass => '▓',
            Block::Dirt => '▒',
            Block::Stone => '▒',
            Block::Cobblestone => '░',
            Block::Sand => '▒',
            Block::Water => '~',
            Block::Wood => '║',
            Block::Leaves => '▒',
            Block::Planks => '═',
            Block::CoalOre => '•',
            Block::IronOre => '•',
            Block::Bedrock => '▓',
            Block::Torch => 'ⵚ',
            Block::StoneBrick => '#',
        }
    }

    /// Single base color for the 3D renderer (face shading is applied on top).
    pub fn color3d(self) -> Rgb {
        match self {
            Block::Air => (0, 0, 0),
            Block::Grass => (106, 170, 64),
            Block::Dirt => (134, 96, 67),
            Block::Stone => (125, 125, 125),
            Block::Cobblestone => (108, 108, 108),
            Block::Sand => (219, 207, 163),
            Block::Water => (47, 93, 222),
            Block::Wood => (104, 82, 50),
            Block::Leaves => (58, 142, 48),
            Block::Planks => (178, 148, 90),
            Block::CoalOre => (66, 66, 70),
            Block::IronOre => (192, 156, 126),
            Block::Bedrock => (48, 48, 52),
            Block::Torch => (255, 216, 128),
            Block::StoneBrick => (142, 142, 146),
        }
    }

    /// (foreground, background). A `None` background means "transparent":
    /// the sky/cave background shows through (used for torches).
    pub fn colors(self) -> (Rgb, Option<Rgb>) {
        match self {
            Block::Air => ((0, 0, 0), None),
            Block::Grass => ((70, 170, 60), Some((90, 72, 40))),
            Block::Dirt => ((125, 88, 52), Some((100, 70, 42))),
            Block::Stone => ((132, 132, 138), Some((104, 104, 110))),
            Block::Cobblestone => ((145, 145, 145), Some((95, 95, 100))),
            Block::Sand => ((228, 208, 142), Some((202, 182, 116))),
            Block::Water => ((110, 160, 235), Some((35, 70, 165))),
            Block::Wood => ((152, 112, 62), Some((112, 82, 46))),
            Block::Leaves => ((45, 125, 45), Some((26, 88, 32))),
            Block::Planks => ((195, 152, 92), Some((162, 122, 72))),
            Block::CoalOre => ((28, 28, 30), Some((104, 104, 110))),
            Block::IronOre => ((225, 185, 152), Some((104, 104, 110))),
            Block::Bedrock => ((62, 62, 66), Some((34, 34, 40))),
            Block::Torch => ((255, 205, 80), None),
            Block::StoneBrick => ((150, 150, 156), Some((112, 112, 120))),
        }
    }
}
