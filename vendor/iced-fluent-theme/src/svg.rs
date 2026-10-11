use crate::Theme;
use iced_widget::svg::{Catalog, Status, Style};

impl Catalog for Theme {
    type Class<'a> = Box<dyn Fn(&Theme, Status) -> Style + 'a>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(none)
    }

    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style {
        class(self, status)
    }
}

pub fn none(_theme: &Theme, _status: Status) -> Style {
    Style { color: None }
}

pub fn neutral_foreground1(theme: &Theme, _status: Status) -> Style {
    Style {
        color: Some(theme.tokens().neutral_foreground1),
    }
}
