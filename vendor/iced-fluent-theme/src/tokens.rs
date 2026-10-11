use crate::{
    BrandVariants,
    color::{
        BLACK, BlackAlpha, CRANBERRY, GREEN, Grey, Grey10Alpha, Grey12Alpha, Grey14Alpha, ORANGE,
        TRANSPARENT, WHITE, WhiteAlpha,
    },
    shadow,
};

use iced_core::{Color, Shadow, color};

// Palette based on Fluent Ui React v9
// https://react.fluentui.dev/?path=/docs/theme-colors--docs

#[derive(Clone, Debug)]
pub struct Tokens {
    pub neutral_foreground1: Color,
    pub neutral_foreground1_hover: Color,
    pub neutral_foreground1_pressed: Color,
    pub neutral_foreground1_selected: Color,

    pub neutral_foreground2: Color,
    pub neutral_foreground2_hover: Color,
    pub neutral_foreground2_pressed: Color,
    pub neutral_foreground2_selected: Color,
    pub neutral_foreground2_brand_hover: Color,
    pub neutral_foreground2_brand_pressed: Color,
    pub neutral_foreground2_brand_selected: Color,

    pub neutral_foreground3: Color,
    pub neutral_foreground3_hover: Color,
    pub neutral_foreground3_pressed: Color,
    pub neutral_foreground3_selected: Color,
    pub neutral_foreground3_brand_hover: Color,
    pub neutral_foreground3_brand_pressed: Color,
    pub neutral_foreground3_brand_selected: Color,

    pub neutral_foreground4: Color,
    pub neutral_foreground_disabled: Color,
    pub neutral_foreground_inverted_disabled: Color,

    pub brand_foreground_link: Color,
    pub brand_foreground_link_hover: Color,
    pub brand_foreground_link_pressed: Color,
    pub brand_foreground_link_selected: Color,

    pub neutral_foreground2_link: Color,
    pub neutral_foreground2_link_hover: Color,
    pub neutral_foreground2_link_pressed: Color,
    pub neutral_foreground2_link_selected: Color,

    pub compound_brand_foreground1: Color,
    pub compound_brand_foreground1_hover: Color,
    pub compound_brand_foreground1_pressed: Color,

    pub brand_foreground1: Color,
    pub brand_foreground2: Color,
    pub brand_foreground2_hover: Color,
    pub brand_foreground2_pressed: Color,

    pub neutral_foreground1_static: Color,

    pub neutral_foreground_static_inverted: Color,
    pub neutral_foreground_inverted: Color,
    pub neutral_foreground_inverted_hover: Color,
    pub neutral_foreground_inverted_pressed: Color,
    pub neutral_foreground_inverted_selected: Color,
    pub neutral_foreground_inverted2: Color,
    pub neutral_foreground_on_brand: Color,
    pub neutral_foreground_inverted_link: Color,
    pub neutral_foreground_inverted_link_hover: Color,
    pub neutral_foreground_inverted_link_pressed: Color,
    pub neutral_foreground_inverted_link_selected: Color,

    pub brand_foreground_inverted: Color,
    pub brand_foreground_inverted_hover: Color,
    pub brand_foreground_inverted_pressed: Color,
    pub brand_foreground_on_light: Color,
    pub brand_foreground_on_light_hover: Color,
    pub brand_foreground_on_light_pressed: Color,
    pub brand_foreground_on_light_selected: Color,

    pub neutral_background1: Color,
    pub neutral_background1_hover: Color,
    pub neutral_background1_pressed: Color,
    pub neutral_background1_selected: Color,

    pub neutral_background2: Color,
    pub neutral_background2_hover: Color,
    pub neutral_background2_pressed: Color,
    pub neutral_background2_selected: Color,

    pub neutral_background3: Color,
    pub neutral_background3_hover: Color,
    pub neutral_background3_pressed: Color,
    pub neutral_background3_selected: Color,

    pub neutral_background4: Color,
    pub neutral_background4_hover: Color,
    pub neutral_background4_pressed: Color,
    pub neutral_background4_selected: Color,

    pub neutral_background5: Color,
    pub neutral_background5_hover: Color,
    pub neutral_background5_pressed: Color,
    pub neutral_background5_selected: Color,

    pub neutral_background6: Color,

    pub neutral_background_inverted: Color,
    pub neutral_background_static: Color,
    pub neutral_background_alpha: Color,
    pub neutral_background_alpha2: Color,

    pub subtle_background: Color,
    pub subtle_background_hover: Color,
    pub subtle_background_pressed: Color,
    pub subtle_background_selected: Color,
    pub subtle_background_light_alpha_hover: Color,
    pub subtle_background_light_alpha_pressed: Color,
    pub subtle_background_light_alpha_selected: Color,

    pub subtle_background_inverted: Color,
    pub subtle_background_inverted_hover: Color,
    pub subtle_background_inverted_pressed: Color,
    pub subtle_background_inverted_selected: Color,

    pub transparent_background: Color,
    pub transparent_background_hover: Color,
    pub transparent_background_pressed: Color,
    pub transparent_background_selected: Color,

    pub neutral_background_disabled: Color,
    pub neutral_background_inverted_disabled: Color,

    pub neutral_stencil1: Color,
    pub neutral_stencil2: Color,
    pub neutral_stencil1_alpha: Color,
    pub neutral_stencil2_alpha: Color,

    pub background_overlay: Color,

    pub scrollbar_overlay: Color,

    pub brand_background: Color,
    pub brand_background_hover: Color,
    pub brand_background_pressed: Color,
    pub brand_background_selected: Color,

    pub compound_brand_background: Color,
    pub compound_brand_background_hover: Color,
    pub compound_brand_background_pressed: Color,

    pub brand_background_static: Color,

    pub brand_background2: Color,
    pub brand_background2_hover: Color,
    pub brand_background2_pressed: Color,

    pub brand_background3_static: Color,

    pub brand_background4_static: Color,

    pub brand_background_inverted: Color,
    pub brand_background_inverted_hover: Color,
    pub brand_background_inverted_pressed: Color,
    pub brand_background_inverted_selected: Color,

    pub neutral_card_background: Color,
    pub neutral_card_background_hover: Color,
    pub neutral_card_background_pressed: Color,
    pub neutral_card_background_selected: Color,
    pub neutral_card_background_disabled: Color,

    pub neutral_stroke_accessible: Color,
    pub neutral_stroke_accessible_hover: Color,
    pub neutral_stroke_accessible_pressed: Color,
    pub neutral_stroke_accessible_selected: Color,

    pub neutral_stroke1: Color,
    pub neutral_stroke1_hover: Color,
    pub neutral_stroke1_pressed: Color,
    pub neutral_stroke1_selected: Color,

    pub neutral_stroke2: Color,

    pub neutral_stroke3: Color,

    pub neutral_stroke_subtle: Color,

    pub neutral_stroke_on_brand: Color,

    pub neutral_stroke_on_brand2: Color,
    pub neutral_stroke_on_brand2_hover: Color,
    pub neutral_stroke_on_brand2_pressed: Color,
    pub neutral_stroke_on_brand2_selected: Color,

    pub brand_stroke1: Color,

    pub brand_stroke2: Color,
    pub brand_stroke2_hover: Color,
    pub brand_stroke2_pressed: Color,
    pub brand_stroke2_contrast: Color,

    pub compound_brand_stroke: Color,
    pub compound_brand_stroke_hover: Color,
    pub compound_brand_stroke_pressed: Color,

    pub neutral_stroke_disabled: Color,
    pub neutral_stroke_inverted_disabled: Color,

    pub transparent_stroke: Color,
    pub transparent_stroke_interactive: Color,
    pub transparent_stroke_disabled: Color,

    pub neutral_stroke_alpha: Color,
    pub neutral_stroke_alpha2: Color,

    pub stroke_focus1: Color,
    pub stroke_focus2: Color,

    pub neutral_shadow_ambient: Color,
    pub neutral_shadow_key: Color,
    pub neutral_shadow_ambient_lighter: Color,
    pub neutral_shadow_key_lighter: Color,
    pub neutral_shadow_ambient_darker: Color,
    pub neutral_shadow_key_darker: Color,

    pub brand_shadow_ambient: Color,
    pub brand_shadow_key: Color,

    pub status_success_background1: Color,
    pub status_success_background2: Color,
    pub status_success_background3: Color,
    pub status_success_foreground1: Color,
    pub status_success_foreground2: Color,
    pub status_success_foreground3: Color,
    pub status_success_foreground_inverted: Color,
    pub status_success_border_active: Color,
    pub status_success_border1: Color,
    pub status_success_border2: Color,

    pub status_warning_background1: Color,
    pub status_warning_background2: Color,
    pub status_warning_background3: Color,
    pub status_warning_foreground1: Color,
    pub status_warning_foreground2: Color,
    pub status_warning_foreground3: Color,
    pub status_warning_foreground_inverted: Color,
    pub status_warning_border_active: Color,
    pub status_warning_border1: Color,
    pub status_warning_border2: Color,

    pub status_danger_background1: Color,
    pub status_danger_background2: Color,
    pub status_danger_background3: Color,
    pub status_danger_foreground1: Color,
    pub status_danger_foreground2: Color,
    pub status_danger_foreground3: Color,
    pub status_danger_foreground_inverted: Color,
    pub status_danger_border_active: Color,
    pub status_danger_border1: Color,
    pub status_danger_border2: Color,

    pub shadow2: Shadow,
    pub shadow4: Shadow,
    pub shadow8: Shadow,
    pub shadow16: Shadow,
    pub shadow28: Shadow,
    pub shadow64: Shadow,

    pub shadow2_brand: Shadow,
    pub shadow4_brand: Shadow,
    pub shadow8_brand: Shadow,
    pub shadow16_brand: Shadow,
    pub shadow28_brand: Shadow,
    pub shadow64_brand: Shadow,
}

impl Tokens {
    // Tokens based on Fluent Ui React v9
    // https://react.fluentui.dev/?path=/docs/theme-colors--docs
    // and
    // https://github.com/microsoft/fluentui/blob/3d2d9bf0e07e05f2320e5400efe6c01759426534/packages/tokens/src/alias/lightColor.ts

    pub const fn light(brand: Option<BrandVariants>) -> Self {
        let brand = match brand {
            Some(brand) => brand,
            None => BrandVariants::DEFAULT,
        };

        Self {
            neutral_foreground1: Grey::G14.color(),
            neutral_foreground1_hover: Grey::G14.color(),
            neutral_foreground1_pressed: Grey::G14.color(),
            neutral_foreground1_selected: Grey::G14.color(),

            neutral_foreground2: Grey::G26.color(),
            neutral_foreground2_hover: Grey::G14.color(),
            neutral_foreground2_pressed: Grey::G14.color(),
            neutral_foreground2_selected: Grey::G14.color(),
            neutral_foreground2_brand_hover: brand.b80,
            neutral_foreground2_brand_pressed: brand.b70,
            neutral_foreground2_brand_selected: brand.b80,

            neutral_foreground3: Grey::G38.color(),
            neutral_foreground3_hover: Grey::G26.color(),
            neutral_foreground3_pressed: Grey::G26.color(),
            neutral_foreground3_selected: Grey::G26.color(),
            neutral_foreground3_brand_hover: brand.b80,
            neutral_foreground3_brand_pressed: brand.b70,
            neutral_foreground3_brand_selected: brand.b80,

            neutral_foreground4: Grey::G44.color(),
            neutral_foreground_disabled: Grey::G74.color(),
            neutral_foreground_inverted_disabled: WhiteAlpha::A40.color(),

            brand_foreground_link: brand.b70,
            brand_foreground_link_hover: brand.b60,
            brand_foreground_link_pressed: brand.b40,
            brand_foreground_link_selected: brand.b70,

            neutral_foreground2_link: Grey::G26.color(),
            neutral_foreground2_link_hover: Grey::G14.color(),
            neutral_foreground2_link_pressed: Grey::G14.color(),
            neutral_foreground2_link_selected: Grey::G14.color(),

            compound_brand_foreground1: brand.b80,
            compound_brand_foreground1_hover: brand.b70,
            compound_brand_foreground1_pressed: brand.b60,

            brand_foreground1: brand.b80,
            brand_foreground2: brand.b70,
            brand_foreground2_hover: brand.b60,
            brand_foreground2_pressed: brand.b30,

            neutral_foreground1_static: Grey::G14.color(),
            neutral_foreground_static_inverted: WHITE,

            neutral_foreground_inverted: WHITE,
            neutral_foreground_inverted_hover: WHITE,
            neutral_foreground_inverted_pressed: WHITE,
            neutral_foreground_inverted_selected: WHITE,
            neutral_foreground_inverted2: WHITE,
            neutral_foreground_on_brand: WHITE,

            neutral_foreground_inverted_link: WHITE,
            neutral_foreground_inverted_link_hover: WHITE,
            neutral_foreground_inverted_link_pressed: WHITE,
            neutral_foreground_inverted_link_selected: WHITE,

            brand_foreground_inverted: brand.b100,
            brand_foreground_inverted_hover: brand.b110,
            brand_foreground_inverted_pressed: brand.b100,

            brand_foreground_on_light: brand.b80,
            brand_foreground_on_light_hover: brand.b70,
            brand_foreground_on_light_pressed: brand.b50,
            brand_foreground_on_light_selected: brand.b60,

            neutral_background1: WHITE,
            neutral_background1_hover: Grey::G96.color(),
            neutral_background1_pressed: Grey::G88.color(),
            neutral_background1_selected: Grey::G92.color(),

            neutral_background2: Grey::G98.color(),
            neutral_background2_hover: Grey::G94.color(),
            neutral_background2_pressed: Grey::G86.color(),
            neutral_background2_selected: Grey::G90.color(),

            neutral_background3: Grey::G96.color(),
            neutral_background3_hover: Grey::G92.color(),
            neutral_background3_pressed: Grey::G84.color(),
            neutral_background3_selected: Grey::G88.color(),

            neutral_background4: Grey::G94.color(),
            neutral_background4_hover: Grey::G98.color(),
            neutral_background4_pressed: Grey::G96.color(),
            neutral_background4_selected: WHITE,

            neutral_background5: Grey::G92.color(),
            neutral_background5_hover: Grey::G96.color(),
            neutral_background5_pressed: Grey::G94.color(),
            neutral_background5_selected: Grey::G98.color(),

            neutral_background6: Grey::G90.color(),
            neutral_background_inverted: Grey::G16.color(),
            neutral_background_static: Grey::G20.color(),

            neutral_background_alpha: WhiteAlpha::A50.color(),
            neutral_background_alpha2: WhiteAlpha::A80.color(),

            subtle_background: TRANSPARENT,
            subtle_background_hover: Grey::G96.color(),
            subtle_background_pressed: Grey::G88.color(),
            subtle_background_selected: Grey::G92.color(),

            subtle_background_light_alpha_hover: WhiteAlpha::A70.color(),
            subtle_background_light_alpha_pressed: WhiteAlpha::A50.color(),
            subtle_background_light_alpha_selected: TRANSPARENT,

            subtle_background_inverted: TRANSPARENT,
            subtle_background_inverted_hover: BlackAlpha::A10.color(),
            subtle_background_inverted_pressed: BlackAlpha::A30.color(),
            subtle_background_inverted_selected: BlackAlpha::A20.color(),

            transparent_background: TRANSPARENT,
            transparent_background_hover: TRANSPARENT,
            transparent_background_pressed: TRANSPARENT,
            transparent_background_selected: TRANSPARENT,

            neutral_background_disabled: Grey::G94.color(),
            neutral_background_inverted_disabled: WhiteAlpha::A10.color(),

            neutral_stencil1: Grey::G90.color(),
            neutral_stencil2: Grey::G98.color(),
            neutral_stencil1_alpha: BlackAlpha::A10.color(),
            neutral_stencil2_alpha: BlackAlpha::A5.color(),

            background_overlay: BlackAlpha::A40.color(),
            scrollbar_overlay: BlackAlpha::A50.color(),

            brand_background: brand.b80,
            brand_background_hover: brand.b70,
            brand_background_pressed: brand.b40,
            brand_background_selected: brand.b60,

            compound_brand_background: brand.b80,
            compound_brand_background_hover: brand.b70,
            compound_brand_background_pressed: brand.b60,

            brand_background_static: brand.b80,
            brand_background2: brand.b160,
            brand_background2_hover: brand.b150,
            brand_background2_pressed: brand.b130,

            brand_background3_static: brand.b60,
            brand_background4_static: brand.b40,

            brand_background_inverted: WHITE,
            brand_background_inverted_hover: brand.b160,
            brand_background_inverted_pressed: brand.b140,
            brand_background_inverted_selected: brand.b150,

            neutral_card_background: Grey::G98.color(),
            neutral_card_background_hover: WHITE,
            neutral_card_background_pressed: Grey::G96.color(),
            neutral_card_background_selected: Grey::G92.color(),
            neutral_card_background_disabled: Grey::G94.color(),

            neutral_stroke_accessible: Grey::G38.color(),
            neutral_stroke_accessible_hover: Grey::G34.color(),
            neutral_stroke_accessible_pressed: Grey::G30.color(),
            neutral_stroke_accessible_selected: brand.b80,

            neutral_stroke1: Grey::G82.color(),
            neutral_stroke1_hover: Grey::G78.color(),
            neutral_stroke1_pressed: Grey::G70.color(),
            neutral_stroke1_selected: Grey::G74.color(),

            neutral_stroke2: Grey::G88.color(),
            neutral_stroke3: Grey::G94.color(),
            neutral_stroke_subtle: Grey::G88.color(),

            neutral_stroke_on_brand: WHITE,
            neutral_stroke_on_brand2: WHITE,
            neutral_stroke_on_brand2_hover: WHITE,
            neutral_stroke_on_brand2_pressed: WHITE,
            neutral_stroke_on_brand2_selected: WHITE,

            brand_stroke1: brand.b80,
            brand_stroke2: brand.b140,
            brand_stroke2_hover: brand.b120,
            brand_stroke2_pressed: brand.b80,
            brand_stroke2_contrast: brand.b140,

            compound_brand_stroke: brand.b80,
            compound_brand_stroke_hover: brand.b70,
            compound_brand_stroke_pressed: brand.b60,

            neutral_stroke_disabled: Grey::G88.color(),
            neutral_stroke_inverted_disabled: WhiteAlpha::A40.color(),

            transparent_stroke: TRANSPARENT,
            transparent_stroke_interactive: TRANSPARENT,
            transparent_stroke_disabled: TRANSPARENT,

            neutral_stroke_alpha: BlackAlpha::A5.color(),
            neutral_stroke_alpha2: WhiteAlpha::A20.color(),

            stroke_focus1: WHITE,
            stroke_focus2: BLACK,

            neutral_shadow_ambient: color!(0, 0, 0, 0.12),
            neutral_shadow_key: color!(0, 0, 0, 0.14),
            neutral_shadow_ambient_lighter: color!(0, 0, 0, 0.06),
            neutral_shadow_key_lighter: color!(0, 0, 0, 0.07),
            neutral_shadow_ambient_darker: color!(0, 0, 0, 0.20),
            neutral_shadow_key_darker: color!(0, 0, 0, 0.24),

            brand_shadow_ambient: color!(0, 0, 0, 0.30),
            brand_shadow_key: color!(0, 0, 0, 0.25),

            status_success_background1: GREEN.tint60,
            status_success_background2: GREEN.tint40,
            status_success_background3: GREEN.primary,
            status_success_foreground1: GREEN.shade10,
            status_success_foreground2: GREEN.shade30,
            status_success_foreground3: GREEN.primary,
            status_success_foreground_inverted: GREEN.tint30,
            status_success_border_active: GREEN.primary,
            status_success_border1: GREEN.tint40,
            status_success_border2: GREEN.primary,

            status_warning_background1: ORANGE.tint60,
            status_warning_background2: ORANGE.tint40,
            status_warning_background3: ORANGE.primary,
            status_warning_foreground1: ORANGE.shade10,
            status_warning_foreground2: ORANGE.shade30,
            status_warning_foreground3: ORANGE.primary,
            status_warning_foreground_inverted: ORANGE.tint30,
            status_warning_border_active: ORANGE.primary,
            status_warning_border1: ORANGE.tint40,
            status_warning_border2: ORANGE.primary,

            status_danger_background1: CRANBERRY.tint60,
            status_danger_background2: CRANBERRY.tint40,
            status_danger_background3: CRANBERRY.primary,
            status_danger_foreground1: CRANBERRY.shade10,
            status_danger_foreground2: CRANBERRY.shade30,
            status_danger_foreground3: CRANBERRY.primary,
            status_danger_foreground_inverted: CRANBERRY.tint30,
            status_danger_border_active: CRANBERRY.primary,
            status_danger_border1: CRANBERRY.tint40,
            status_danger_border2: CRANBERRY.primary,

            shadow2: shadow::LIGHT.shadow2,
            shadow4: shadow::LIGHT.shadow4,
            shadow8: shadow::LIGHT.shadow8,
            shadow16: shadow::LIGHT.shadow16,
            shadow28: shadow::LIGHT.shadow28,
            shadow64: shadow::LIGHT.shadow64,

            shadow2_brand: shadow::LIGHT.shadow2_brand,
            shadow4_brand: shadow::LIGHT.shadow4_brand,
            shadow8_brand: shadow::LIGHT.shadow8_brand,
            shadow16_brand: shadow::LIGHT.shadow16_brand,
            shadow28_brand: shadow::LIGHT.shadow28_brand,
            shadow64_brand: shadow::LIGHT.shadow64_brand,
        }
    }

    // Tokens based on Fluent Ui React v9
    // https://react.fluentui.dev/?path=/docs/theme-colors--docs
    // and
    // https://github.com/microsoft/fluentui/blob/3d2d9bf0e07e05f2320e5400efe6c01759426534/packages/tokens/src/alias/darkColor.ts

    pub const fn dark(brand: Option<BrandVariants>) -> Self {
        let brand = match brand {
            Some(brand) => brand,
            None => BrandVariants::DEFAULT,
        };

        Tokens {
            neutral_foreground1: WHITE,
            neutral_foreground1_hover: WHITE,
            neutral_foreground1_pressed: WHITE,
            neutral_foreground1_selected: WHITE,

            neutral_foreground2: Grey::G84.color(),
            neutral_foreground2_hover: WHITE,
            neutral_foreground2_pressed: WHITE,
            neutral_foreground2_selected: WHITE,
            neutral_foreground2_brand_hover: brand.b100,
            neutral_foreground2_brand_pressed: brand.b90,
            neutral_foreground2_brand_selected: brand.b100,

            neutral_foreground3: Grey::G68.color(),
            neutral_foreground3_hover: Grey::G84.color(),
            neutral_foreground3_pressed: Grey::G84.color(),
            neutral_foreground3_selected: Grey::G84.color(),
            neutral_foreground3_brand_hover: brand.b100,
            neutral_foreground3_brand_pressed: brand.b90,
            neutral_foreground3_brand_selected: brand.b100,

            neutral_foreground4: Grey::G60.color(),

            neutral_foreground_disabled: Grey::G36.color(),
            neutral_foreground_inverted_disabled: WhiteAlpha::A40.color(),

            brand_foreground_link: brand.b100,
            brand_foreground_link_hover: brand.b110,
            brand_foreground_link_pressed: brand.b90,
            brand_foreground_link_selected: brand.b100,

            neutral_foreground2_link: Grey::G84.color(),
            neutral_foreground2_link_hover: WHITE,
            neutral_foreground2_link_pressed: WHITE,
            neutral_foreground2_link_selected: WHITE,

            compound_brand_foreground1: brand.b100,
            compound_brand_foreground1_hover: brand.b110,
            compound_brand_foreground1_pressed: brand.b90,

            brand_foreground1: brand.b100,
            brand_foreground2: brand.b110,
            brand_foreground2_hover: brand.b130,
            brand_foreground2_pressed: brand.b160,

            neutral_foreground1_static: Grey::G14.color(),
            neutral_foreground_static_inverted: WHITE,

            neutral_foreground_inverted: Grey::G14.color(),
            neutral_foreground_inverted_hover: Grey::G14.color(),
            neutral_foreground_inverted_pressed: Grey::G14.color(),
            neutral_foreground_inverted_selected: Grey::G14.color(),

            neutral_foreground_inverted2: Grey::G14.color(),
            neutral_foreground_on_brand: WHITE,

            neutral_foreground_inverted_link: WHITE,
            neutral_foreground_inverted_link_hover: WHITE,
            neutral_foreground_inverted_link_pressed: WHITE,
            neutral_foreground_inverted_link_selected: WHITE,

            brand_foreground_inverted: brand.b80,
            brand_foreground_inverted_hover: brand.b70,
            brand_foreground_inverted_pressed: brand.b60,

            brand_foreground_on_light: brand.b80,
            brand_foreground_on_light_hover: brand.b70,
            brand_foreground_on_light_pressed: brand.b50,
            brand_foreground_on_light_selected: brand.b60,

            neutral_background1: Grey::G16.color(),
            neutral_background1_hover: Grey::G24.color(),
            neutral_background1_pressed: Grey::G12.color(),
            neutral_background1_selected: Grey::G22.color(),

            neutral_background2: Grey::G12.color(),
            neutral_background2_hover: Grey::G20.color(),
            neutral_background2_pressed: Grey::G8.color(),
            neutral_background2_selected: Grey::G18.color(),

            neutral_background3: Grey::G8.color(),
            neutral_background3_hover: Grey::G16.color(),
            neutral_background3_pressed: Grey::G4.color(),
            neutral_background3_selected: Grey::G14.color(),

            neutral_background4: Grey::G4.color(),
            neutral_background4_hover: Grey::G12.color(),
            neutral_background4_pressed: BLACK,
            neutral_background4_selected: Grey::G10.color(),

            neutral_background5: BLACK,
            neutral_background5_hover: Grey::G8.color(),
            neutral_background5_pressed: Grey::G2.color(),
            neutral_background5_selected: Grey::G6.color(),

            neutral_background6: Grey::G20.color(),

            neutral_background_inverted: WHITE,
            neutral_background_static: Grey::G24.color(),

            neutral_background_alpha: Grey10Alpha::A50.color(),
            neutral_background_alpha2: Grey12Alpha::A70.color(),

            subtle_background: TRANSPARENT,
            subtle_background_hover: Grey::G22.color(),
            subtle_background_pressed: Grey::G18.color(),
            subtle_background_selected: Grey::G20.color(),

            subtle_background_light_alpha_hover: Grey14Alpha::A80.color(),
            subtle_background_light_alpha_pressed: Grey14Alpha::A50.color(),
            subtle_background_light_alpha_selected: TRANSPARENT,

            subtle_background_inverted: TRANSPARENT,
            subtle_background_inverted_hover: BlackAlpha::A10.color(),
            subtle_background_inverted_pressed: BlackAlpha::A30.color(),
            subtle_background_inverted_selected: BlackAlpha::A20.color(),

            transparent_background: TRANSPARENT,
            transparent_background_hover: TRANSPARENT,
            transparent_background_pressed: TRANSPARENT,
            transparent_background_selected: TRANSPARENT,

            neutral_background_disabled: Grey::G8.color(),
            neutral_background_inverted_disabled: WhiteAlpha::A10.color(),

            neutral_stencil1: Grey::G34.color(),
            neutral_stencil2: Grey::G20.color(),

            neutral_stencil1_alpha: WhiteAlpha::A10.color(),
            neutral_stencil2_alpha: WhiteAlpha::A5.color(),

            background_overlay: BlackAlpha::A50.color(),
            scrollbar_overlay: WhiteAlpha::A60.color(),

            brand_background: brand.b70,
            brand_background_hover: brand.b80,
            brand_background_pressed: brand.b40,
            brand_background_selected: brand.b60,

            compound_brand_background: brand.b100,
            compound_brand_background_hover: brand.b110,
            compound_brand_background_pressed: brand.b90,

            brand_background_static: brand.b80,

            brand_background2: brand.b20,
            brand_background2_hover: brand.b40,
            brand_background2_pressed: brand.b10,

            brand_background3_static: brand.b60,
            brand_background4_static: brand.b40,

            brand_background_inverted: WHITE,
            brand_background_inverted_hover: brand.b160,
            brand_background_inverted_pressed: brand.b140,
            brand_background_inverted_selected: brand.b150,

            neutral_card_background: Grey::G20.color(),
            neutral_card_background_hover: Grey::G24.color(),
            neutral_card_background_pressed: Grey::G18.color(),
            neutral_card_background_selected: Grey::G22.color(),
            neutral_card_background_disabled: Grey::G8.color(),

            neutral_stroke_accessible: Grey::G68.color(),
            neutral_stroke_accessible_hover: Grey::G74.color(),
            neutral_stroke_accessible_pressed: Grey::G70.color(),
            neutral_stroke_accessible_selected: brand.b100,

            neutral_stroke1: Grey::G40.color(),
            neutral_stroke1_hover: Grey::G46.color(),
            neutral_stroke1_pressed: Grey::G42.color(),
            neutral_stroke1_selected: Grey::G44.color(),

            neutral_stroke2: Grey::G32.color(),
            neutral_stroke3: Grey::G24.color(),
            neutral_stroke_subtle: Grey::G4.color(),

            neutral_stroke_on_brand: Grey::G16.color(),

            neutral_stroke_on_brand2: WHITE,
            neutral_stroke_on_brand2_hover: WHITE,
            neutral_stroke_on_brand2_pressed: WHITE,
            neutral_stroke_on_brand2_selected: WHITE,

            brand_stroke1: brand.b100,
            brand_stroke2: brand.b50,
            brand_stroke2_hover: brand.b50,
            brand_stroke2_pressed: brand.b30,
            brand_stroke2_contrast: brand.b50,

            compound_brand_stroke: brand.b100,
            compound_brand_stroke_hover: brand.b110,
            compound_brand_stroke_pressed: brand.b90,

            neutral_stroke_disabled: Grey::G26.color(),
            neutral_stroke_inverted_disabled: WhiteAlpha::A40.color(),

            transparent_stroke: TRANSPARENT,
            transparent_stroke_interactive: TRANSPARENT,
            transparent_stroke_disabled: TRANSPARENT,

            neutral_stroke_alpha: WhiteAlpha::A10.color(),
            neutral_stroke_alpha2: WhiteAlpha::A20.color(),

            stroke_focus1: BLACK,
            stroke_focus2: WHITE,

            neutral_shadow_ambient: color!(0, 0, 0, 0.24),
            neutral_shadow_key: color!(0, 0, 0, 0.28),
            neutral_shadow_ambient_lighter: color!(0, 0, 0, 0.12),
            neutral_shadow_key_lighter: color!(0, 0, 0, 0.14),
            neutral_shadow_ambient_darker: color!(0, 0, 0, 0.4),
            neutral_shadow_key_darker: color!(0, 0, 0, 0.48),

            brand_shadow_ambient: color!(0, 0, 0, 0.30),
            brand_shadow_key: color!(0, 0, 0, 0.25),

            status_success_background1: GREEN.shade40,
            status_success_background2: GREEN.shade30,
            status_success_background3: GREEN.primary,
            status_success_foreground1: GREEN.tint30,
            status_success_foreground2: GREEN.tint40,
            status_success_foreground3: GREEN.tint20,
            status_success_foreground_inverted: GREEN.shade10,
            status_success_border_active: GREEN.tint30,
            status_success_border1: GREEN.primary,
            status_success_border2: GREEN.tint20,

            status_warning_background1: ORANGE.shade40,
            status_warning_background2: ORANGE.shade30,
            status_warning_background3: ORANGE.primary,
            status_warning_foreground1: ORANGE.tint30,
            status_warning_foreground2: ORANGE.tint40,
            status_warning_foreground3: ORANGE.tint20,
            status_warning_foreground_inverted: ORANGE.shade10,
            status_warning_border_active: ORANGE.tint30,
            status_warning_border1: ORANGE.primary,
            status_warning_border2: ORANGE.tint20,

            status_danger_background1: CRANBERRY.shade40,
            status_danger_background2: CRANBERRY.shade30,
            status_danger_background3: CRANBERRY.primary,
            status_danger_foreground1: CRANBERRY.tint30,
            status_danger_foreground2: CRANBERRY.tint40,
            status_danger_foreground3: CRANBERRY.tint20,
            status_danger_foreground_inverted: CRANBERRY.shade10,
            status_danger_border_active: CRANBERRY.tint30,
            status_danger_border1: CRANBERRY.primary,
            status_danger_border2: CRANBERRY.tint20,

            shadow2: shadow::DARK.shadow2,
            shadow4: shadow::DARK.shadow4,
            shadow8: shadow::DARK.shadow8,
            shadow16: shadow::DARK.shadow16,
            shadow28: shadow::DARK.shadow28,
            shadow64: shadow::DARK.shadow64,

            shadow2_brand: shadow::DARK.shadow2_brand,
            shadow4_brand: shadow::DARK.shadow4_brand,
            shadow8_brand: shadow::DARK.shadow8_brand,
            shadow16_brand: shadow::DARK.shadow16_brand,
            shadow28_brand: shadow::DARK.shadow28_brand,
            shadow64_brand: shadow::DARK.shadow64_brand,
        }
    }
}
