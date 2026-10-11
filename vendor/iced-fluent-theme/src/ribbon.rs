pub mod button;

use crate::{
    Theme, border_radius,
    button::rounded::subtle,
    font::{self, line_height, size},
};

use iced_core::{
    Border, Element, Font, Shadow,
    text::{IntoFragment, Wrapping},
};

use iced_widget_kit::ribbon::{Catalog, Group, Size, Style, StyleFn};

impl Catalog for Theme {
    type Class<'a> = StyleFn<'a, Self>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(|theme| {
            let tokens = theme.tokens();

            Style {
                background: Some(tokens.neutral_background1.into()),
                border: Border {
                    radius: border_radius::LARGE,
                    ..Border::default()
                },
                shadow: Shadow::default(),
                separator_color: Some(tokens.neutral_stroke1),
                snap: true,
            }
        })
    }

    fn style(&self, class: &Self::Class<'_>) -> Style {
        class(self)
    }
}

pub fn group<'a, Id, Message, Renderer>(
    id: Id,
    header: impl IntoFragment<'a>,
    content: impl Fn(Size) -> Option<Element<'a, Message, Theme, Renderer>> + 'a,
) -> Group<'a, Id, Message, Theme, Renderer>
where
    Id: Clone + Eq,
    Message: 'a + Clone,
    Renderer: 'a + iced_core::Renderer + iced_core::svg::Renderer + iced_core::text::Renderer,
    <Renderer as iced_core::text::Renderer>::Font: From<Font>,
    <Renderer as iced_core::text::Renderer>::Paragraph: Clone,
{
    Group::new(id, header, content)
        .header_font(font::REGULAR)
        .header_size(size::BASE200)
        .header_line_height(line_height::BASE200)
        .header_wrapping(Wrapping::None)
        .collapsed_style(subtle)
}
