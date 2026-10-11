use crate::{Theme, container};
use iced_widget::{container as iced_container, markdown::Catalog};

impl Catalog for Theme {
    fn code_block<'a>() -> <Self as iced_container::Catalog>::Class<'a> {
        Box::new(container::code_block)
    }
}
