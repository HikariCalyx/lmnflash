# Iced Fluent Theme

A [Fluent 2](https://fluent2.microsoft.design) inspired theme for the [Iced](https://github.com/hecrj/iced) GUI library.

It provides styles for widgets from the following crates:
* [iced](https://github.com/hecrj/iced)
* [iced_aw](https://github.com/iced-rs/iced_aw)
* [iced-widget-kit](https://codeberg.org/frgp42/iced-widget-kit.git)

## Usage

Include `iced-fluent-theme` as a dependency in your `Cargo.toml`:

```toml
[dependencies]
iced = "0.14.0"
iced-fluent-theme = "0.1.0"
```

When using this theme, notify iced of the built-in font in your settings:
```rust
let settings = iced::Settings {
    fonts: iced-fluent-theme::font::load(),
    default_font: iced-fluent-theme::font::REGULAR,
    ..Default::default()
};
```

This theme currently comes in two variants: Light and Dark:
```rust
pub enum Theme {
    Light(Tokens),
    Dark(Tokens),
}
```

To create the theme requires providing the relevant constructors with a color ramp which are
then used by the theme to create the brand color tokens:
```rust
const BRAND: BrandVariants = BrandVariants {
    b10: color!(0x020305),
    b20: color!(0x121725),
    b30: color!(0x192542),
    b40: color!(0x1D305A),
    b50: color!(0x1F3C73),
    b60: color!(0x21498D),
    b70: color!(0x2156A8),
    b80: color!(0x1E63C4),
    b90: color!(0x1970E1),
    b100: color!(0x0C7EFE),
    b110: color!(0x4D8CFF),
    b120: color!(0x6F9AFF),
    b130: color!(0x89A9FF),
    b140: color!(0xA1B8FF),
    b150: color!(0xB7C7FF),
    b160: color!(0xCCD6FF),
};

let light_theme = Theme::light(Some(BRAND));
let dark_theme = Theme::dark(Some(BRAND));
```

This color ramp can be generated manually or by using the [Fluent Theme Designer](https://storybooks.fluentui.dev/react/?path=/docs/theme-theme-designer--docs).

You can now use the theme with your iced app:
```rust
iced::application(MyApp::new, MyApp::update, MyApp::view)
    .settings(settings)
    .title("My App")
    .theme(light_theme)
    .run()
```

For details on how to use the contained styles and their appearance check out the gallery example:
```console
cargo run -p gallery
```

## Versioning

| `iced-fluent-theme` version | `iced` version  |
| --------------------------- | --------------- |
| 0.1.x                       | 0.14            |

## Disclaimer

This project is independently developed and is not affiliated with, endorsed by, or sponsored by Microsoft.
