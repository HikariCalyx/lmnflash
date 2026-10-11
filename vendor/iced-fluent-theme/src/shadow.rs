use iced_core::{Shadow as IcedShadow, Vector, color};

// Shadow specifications based on
// https://storybooks.fluentui.dev/react/?path=/docs/theme-shadows--docs
//
// Iced only allows the use of a single shadow so only key shadows are used and not ambient shadows

pub struct Shadow {
    pub shadow2: IcedShadow,
    pub shadow4: IcedShadow,
    pub shadow8: IcedShadow,
    pub shadow16: IcedShadow,
    pub shadow28: IcedShadow,
    pub shadow64: IcedShadow,
    pub shadow2_brand: IcedShadow,
    pub shadow4_brand: IcedShadow,
    pub shadow8_brand: IcedShadow,
    pub shadow16_brand: IcedShadow,
    pub shadow28_brand: IcedShadow,
    pub shadow64_brand: IcedShadow,
}

const fn shadow(blur_radius: f32, a: f32) -> IcedShadow {
    IcedShadow {
        color: color!(0x000000, a),
        offset: Vector {
            x: 0.0,
            y: 0.5 * blur_radius,
        },
        blur_radius,
    }
}

pub const LIGHT: Shadow = Shadow {
    shadow2: shadow(2.0, 0.14),
    shadow4: shadow(4.0, 0.14),
    shadow8: shadow(8.0, 0.14),
    shadow16: shadow(16.0, 0.14),
    shadow28: shadow(28.0, 0.14),
    shadow64: shadow(64.0, 0.14),
    shadow2_brand: shadow(2.0, 0.25),
    shadow4_brand: shadow(4.0, 0.25),
    shadow8_brand: shadow(8.0, 0.25),
    shadow16_brand: shadow(16.0, 0.25),
    shadow28_brand: shadow(28.0, 0.25),
    shadow64_brand: shadow(64.0, 0.25),
};

pub const DARK: Shadow = Shadow {
    shadow2: shadow(2.0, 0.28),
    shadow4: shadow(4.0, 0.28),
    shadow8: shadow(8.0, 0.28),
    shadow16: shadow(16.0, 0.28),
    shadow28: shadow(28.0, 0.28),
    shadow64: shadow(64.0, 0.28),
    shadow2_brand: shadow(2.0, 0.25),
    shadow4_brand: shadow(4.0, 0.25),
    shadow8_brand: shadow(8.0, 0.25),
    shadow16_brand: shadow(16.0, 0.25),
    shadow28_brand: shadow(28.0, 0.25),
    shadow64_brand: shadow(64.0, 0.25),
};
