use std::borrow::Cow;

use iced_core::{
    Font,
    font::{Family, Weight},
};

pub const REGULAR: Font = Font::with_name("Inter");

pub const SEMIBOLD: Font = Font {
    family: Family::Name("Inter"),
    weight: Weight::Semibold,
    ..Font::DEFAULT
};

pub const BOLD: Font = Font {
    family: Family::Name("Inter"),
    weight: Weight::Bold,
    ..Font::DEFAULT
};

pub const ICONS: Font = Font::with_name("Icons");

pub fn load() -> Vec<Cow<'static, [u8]>> {
    vec![
        include_bytes!("../assets/font/Inter-VariableFont_opsz,wght.ttf")
            .as_slice()
            .into(),
        include_bytes!("../assets/font/icons.ttf").as_slice().into(),
    ]
}

// Size and line heights from
// https://github.com/microsoft/fluentui/blob/03f737b707ea3867f079b05c239f2a2dbb30b4db/packages/tokens/src/global/fonts.ts#L3

pub mod size {
    use iced_core::Pixels;

    pub const BASE100: Pixels = Pixels(10.0);
    pub const BASE200: Pixels = Pixels(12.0);
    pub const BASE300: Pixels = Pixels(14.0);
    pub const BASE400: Pixels = Pixels(16.0);
    pub const BASE500: Pixels = Pixels(20.0);
    pub const BASE600: Pixels = Pixels(24.0);

    pub const HERO700: Pixels = Pixels(28.0);
    pub const HERO800: Pixels = Pixels(32.0);
    pub const HERO900: Pixels = Pixels(40.0);
    pub const HERO1000: Pixels = Pixels(68.0);
}

pub mod line_height {
    use iced_core::{Pixels, widget::text::LineHeight};

    pub const BASE100: LineHeight = LineHeight::Absolute(Pixels(14.0));
    pub const BASE200: LineHeight = LineHeight::Absolute(Pixels(16.0));
    pub const BASE300: LineHeight = LineHeight::Absolute(Pixels(20.0));
    pub const BASE400: LineHeight = LineHeight::Absolute(Pixels(22.0));
    pub const BASE500: LineHeight = LineHeight::Absolute(Pixels(28.0));
    pub const BASE600: LineHeight = LineHeight::Absolute(Pixels(32.0));

    pub const HERO700: LineHeight = LineHeight::Absolute(Pixels(36.0));
    pub const HERO800: LineHeight = LineHeight::Absolute(Pixels(40.0));
    pub const HERO900: LineHeight = LineHeight::Absolute(Pixels(52.0));
    pub const HERO1000: LineHeight = LineHeight::Absolute(Pixels(92.0));
}
