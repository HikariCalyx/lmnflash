use crate::{Theme, button::rounded::subtle, spacing, text::caption_1};
use iced_core::{
    Element, Font, Length,
    alignment::{Horizontal, Vertical},
};

use iced_widget::{
    Button, button, column, container, row,
    text::{self, IntoFragment},
};

pub mod image {
    use iced_widget::Image;

    pub fn small<Handle>(handle: impl Into<Handle>) -> Image<Handle> {
        Image::new(handle.into()).width(20).height(20)
    }

    pub fn large<Handle>(handle: impl Into<Handle>) -> Image<Handle> {
        Image::new(handle.into()).width(40).height(40)
    }
}

pub mod svg {
    use iced_widget::svg as iced_svg;
    use iced_widget::svg::{Catalog, Handle, Svg};

    pub fn small<'a, Theme>(handle: impl Into<Handle>) -> Svg<'a, Theme>
    where
        Theme: Catalog,
    {
        iced_svg(handle).width(20).height(20)
    }

    pub fn large<'a, Theme>(handle: impl Into<Handle>) -> Svg<'a, Theme>
    where
        Theme: Catalog,
    {
        iced_svg(handle).width(40).height(40)
    }
}

pub fn large<'a, Message, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    label: impl IntoFragment<'a>,
) -> Button<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a + button::Catalog + container::Catalog + text::Catalog,
    Renderer: 'a + iced_core::Renderer + iced_core::text::Renderer,
    <Renderer as iced_core::text::Renderer>::Font: From<Font>,
{
    let content = column![content.into(), caption_1(label)].align_x(Horizontal::Center);

    Button::new(content)
        .width(44)
        .height(76)
        .style(subtle)
        .padding([spacing::NONE.0, spacing::XXS.0])
}

pub fn medium<'a, Message, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    label: impl IntoFragment<'a>,
) -> Button<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a + button::Catalog + text::Catalog,
    Renderer: 'a + iced_core::Renderer + iced_core::text::Renderer,
    <Renderer as iced_core::text::Renderer>::Font: From<Font>,
{
    let content = row![
        content.into(),
        caption_1(label)
            .height(Length::Fill)
            .align_y(Vertical::Center)
    ]
    .align_y(Vertical::Center)
    .spacing(4);

    Button::new(content)
        .width(Length::Shrink)
        .height(24)
        .padding([spacing::NONE.0, spacing::M.0])
        .style(subtle)
}

pub fn small<'a, Message, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> Button<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a + button::Catalog,
    Renderer: 'a + iced_core::Renderer,
{
    Button::new(content.into())
        .width(Length::Shrink)
        .height(24)
        .padding([spacing::NONE.0, spacing::M.0])
        .style(subtle)
}
