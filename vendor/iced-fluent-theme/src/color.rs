use iced_core::Color;
use iced_core::color;

// Colors based on:
// https://github.com/microsoft/fluentui/blob/3d2d9bf0e07e05f2320e5400efe6c01759426534/packages/tokens/src/global/colors.ts

#[derive(Debug, Clone, Copy)]
pub enum Grey {
    G2,
    G4,
    G6,
    G8,
    G10,
    G12,
    G14,
    G16,
    G18,
    G20,
    G22,
    G24,
    G26,
    G28,
    G30,
    G32,
    G34,
    G36,
    G38,
    G40,
    G42,
    G44,
    G46,
    G48,
    G50,
    G52,
    G54,
    G56,
    G58,
    G60,
    G62,
    G64,
    G66,
    G68,
    G70,
    G72,
    G74,
    G76,
    G78,
    G80,
    G82,
    G84,
    G86,
    G88,
    G90,
    G92,
    G94,
    G96,
    G98,
}

impl Grey {
    pub const fn color(self) -> Color {
        match self {
            Grey::G2 => color!(0x050505),
            Grey::G4 => color!(0x0a0a0a),
            Grey::G6 => color!(0x0f0f0f),
            Grey::G8 => color!(0x141414),
            Grey::G10 => color!(0x1a1a1a),
            Grey::G12 => color!(0x1f1f1f),
            Grey::G14 => color!(0x242424),
            Grey::G16 => color!(0x292929),
            Grey::G18 => color!(0x2e2e2e),
            Grey::G20 => color!(0x333333),
            Grey::G22 => color!(0x383838),
            Grey::G24 => color!(0x3d3d3d),
            Grey::G26 => color!(0x424242),
            Grey::G28 => color!(0x474747),
            Grey::G30 => color!(0x4d4d4d),
            Grey::G32 => color!(0x525252),
            Grey::G34 => color!(0x575757),
            Grey::G36 => color!(0x5c5c5c),
            Grey::G38 => color!(0x616161),
            Grey::G40 => color!(0x666666),
            Grey::G42 => color!(0x6b6b6b),
            Grey::G44 => color!(0x707070),
            Grey::G46 => color!(0x757575),
            Grey::G48 => color!(0x7a7a7a),
            Grey::G50 => color!(0x808080),
            Grey::G52 => color!(0x858585),
            Grey::G54 => color!(0x8a8a8a),
            Grey::G56 => color!(0x8f8f8f),
            Grey::G58 => color!(0x949494),
            Grey::G60 => color!(0x999999),
            Grey::G62 => color!(0x9e9e9e),
            Grey::G64 => color!(0xa3a3a3),
            Grey::G66 => color!(0xa8a8a8),
            Grey::G68 => color!(0xadadad),
            Grey::G70 => color!(0xb3b3b3),
            Grey::G72 => color!(0xb8b8b8),
            Grey::G74 => color!(0xbdbdbd),
            Grey::G76 => color!(0xc2c2c2),
            Grey::G78 => color!(0xc7c7c7),
            Grey::G80 => color!(0xcccccc),
            Grey::G82 => color!(0xd1d1d1),
            Grey::G84 => color!(0xd6d6d6),
            Grey::G86 => color!(0xdbdbdb),
            Grey::G88 => color!(0xe0e0e0),
            Grey::G90 => color!(0xe6e6e6),
            Grey::G92 => color!(0xebebeb),
            Grey::G94 => color!(0xf0f0f0),
            Grey::G96 => color!(0xf5f5f5),
            Grey::G98 => color!(0xfafafa),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum WhiteAlpha {
    A5,
    A10,
    A20,
    A30,
    A40,
    A50,
    A60,
    A70,
    A80,
    A90,
}

impl WhiteAlpha {
    pub const fn color(self) -> Color {
        match self {
            WhiteAlpha::A5 => color!(0xffffff, 0.05),
            WhiteAlpha::A10 => color!(0xffffff, 0.10),
            WhiteAlpha::A20 => color!(0xffffff, 0.20),
            WhiteAlpha::A30 => color!(0xffffff, 0.30),
            WhiteAlpha::A40 => color!(0xffffff, 0.40),
            WhiteAlpha::A50 => color!(0xffffff, 0.50),
            WhiteAlpha::A60 => color!(0xffffff, 0.60),
            WhiteAlpha::A70 => color!(0xffffff, 0.70),
            WhiteAlpha::A80 => color!(0xffffff, 0.80),
            WhiteAlpha::A90 => color!(0xffffff, 0.90),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum BlackAlpha {
    A5,
    A10,
    A20,
    A30,
    A40,
    A50,
    A60,
    A70,
    A80,
    A90,
}

impl BlackAlpha {
    pub const fn color(self) -> Color {
        match self {
            BlackAlpha::A5 => color!(0x000000, 0.05),
            BlackAlpha::A10 => color!(0x000000, 0.10),
            BlackAlpha::A20 => color!(0x000000, 0.20),
            BlackAlpha::A30 => color!(0x000000, 0.30),
            BlackAlpha::A40 => color!(0x000000, 0.40),
            BlackAlpha::A50 => color!(0x000000, 0.50),
            BlackAlpha::A60 => color!(0x000000, 0.60),
            BlackAlpha::A70 => color!(0x000000, 0.70),
            BlackAlpha::A80 => color!(0x000000, 0.80),
            BlackAlpha::A90 => color!(0x000000, 0.90),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Grey10Alpha {
    A5,
    A10,
    A20,
    A30,
    A40,
    A50,
    A60,
    A70,
    A80,
    A90,
}

impl Grey10Alpha {
    pub const fn color(self) -> Color {
        match self {
            Grey10Alpha::A5 => color!(0x1a1a1a, 0.05),
            Grey10Alpha::A10 => color!(0x1a1a1a, 0.10),
            Grey10Alpha::A20 => color!(0x1a1a1a, 0.20),
            Grey10Alpha::A30 => color!(0x1a1a1a, 0.30),
            Grey10Alpha::A40 => color!(0x1a1a1a, 0.40),
            Grey10Alpha::A50 => color!(0x1a1a1a, 0.50),
            Grey10Alpha::A60 => color!(0x1a1a1a, 0.60),
            Grey10Alpha::A70 => color!(0x1a1a1a, 0.70),
            Grey10Alpha::A80 => color!(0x1a1a1a, 0.80),
            Grey10Alpha::A90 => color!(0x1a1a1a, 0.90),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Grey12Alpha {
    A5,
    A10,
    A20,
    A30,
    A40,
    A50,
    A60,
    A70,
    A80,
    A90,
}

impl Grey12Alpha {
    pub const fn color(self) -> Color {
        match self {
            Grey12Alpha::A5 => color!(0x1f1f1f, 0.05),
            Grey12Alpha::A10 => color!(0x1f1f1f, 0.10),
            Grey12Alpha::A20 => color!(0x1f1f1f, 0.20),
            Grey12Alpha::A30 => color!(0x1f1f1f, 0.30),
            Grey12Alpha::A40 => color!(0x1f1f1f, 0.40),
            Grey12Alpha::A50 => color!(0x1f1f1f, 0.50),
            Grey12Alpha::A60 => color!(0x1f1f1f, 0.60),
            Grey12Alpha::A70 => color!(0x1f1f1f, 0.70),
            Grey12Alpha::A80 => color!(0x1f1f1f, 0.80),
            Grey12Alpha::A90 => color!(0x1f1f1f, 0.90),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Grey14Alpha {
    A5,
    A10,
    A20,
    A30,
    A40,
    A50,
    A60,
    A70,
    A80,
    A90,
}

impl Grey14Alpha {
    pub const fn color(self) -> Color {
        match self {
            Grey14Alpha::A5 => color!(0x242424, 0.05),
            Grey14Alpha::A10 => color!(0x242424, 0.10),
            Grey14Alpha::A20 => color!(0x242424, 0.20),
            Grey14Alpha::A30 => color!(0x242424, 0.30),
            Grey14Alpha::A40 => color!(0x242424, 0.40),
            Grey14Alpha::A50 => color!(0x242424, 0.50),
            Grey14Alpha::A60 => color!(0x242424, 0.60),
            Grey14Alpha::A70 => color!(0x242424, 0.70),
            Grey14Alpha::A80 => color!(0x242424, 0.80),
            Grey14Alpha::A90 => color!(0x242424, 0.90),
        }
    }
}

pub const WHITE: Color = Color::WHITE;
pub const BLACK: Color = Color::BLACK;
pub const TRANSPARENT: Color = Color::TRANSPARENT;

pub struct ColorVariant {
    pub shade50: Color,
    pub shade40: Color,
    pub shade30: Color,
    pub shade20: Color,
    pub shade10: Color,
    pub primary: Color,
    pub tint10: Color,
    pub tint20: Color,
    pub tint30: Color,
    pub tint40: Color,
    pub tint50: Color,
    pub tint60: Color,
}

pub const CRANBERRY: ColorVariant = ColorVariant {
    shade50: color!(0x200205),
    shade40: color!(0x3b0509),
    shade30: color!(0x6e0811),
    shade20: color!(0x960b18),
    shade10: color!(0xb10e1c),
    primary: color!(0xc50f1f),
    tint10: color!(0xcc2635),
    tint20: color!(0xd33f4c),
    tint30: color!(0xdc626d),
    tint40: color!(0xeeacb2),
    tint50: color!(0xf6d1d5),
    tint60: color!(0xfdf3f4),
};

pub const GREEN: ColorVariant = ColorVariant {
    shade50: color!(0x031403),
    shade40: color!(0x052505),
    shade30: color!(0x094509),
    shade20: color!(0x0c5e0c),
    shade10: color!(0x0e700e),
    primary: color!(0x107c10),
    tint10: color!(0x218c21),
    tint20: color!(0x359b35),
    tint30: color!(0x54b054),
    tint40: color!(0x9fd89f),
    tint50: color!(0xc9eac9),
    tint60: color!(0xf1faf1),
};

pub const ORANGE: ColorVariant = ColorVariant {
    shade50: color!(0x271002),
    shade40: color!(0x4a1e04),
    shade30: color!(0x8a3707),
    shade20: color!(0xbc4b09),
    shade10: color!(0xde590b),
    primary: color!(0xf7630c),
    tint10: color!(0xf87528),
    tint20: color!(0xf98845),
    tint30: color!(0xfaa06b),
    tint40: color!(0xfdcfb4),
    tint50: color!(0xfee5d7),
    tint60: color!(0xfff9f5),
};
