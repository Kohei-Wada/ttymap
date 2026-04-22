//! List and ListItem descriptors.

use ratatui::text::Span as RSpan;
use ratatui::widgets::{List as RList, ListItem as RListItem};

use super::style::TextStyle;

#[derive(Clone, Debug)]
pub struct ListItem {
    pub text: String,
    pub style: TextStyle,
}

impl ListItem {
    pub fn new(text: impl Into<String>, style: TextStyle) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }
}

impl From<ListItem> for RListItem<'static> {
    fn from(item: ListItem) -> Self {
        RListItem::new(RSpan::styled(item.text, item.style))
    }
}

#[derive(Clone, Debug, Default)]
pub struct List {
    pub items: Vec<ListItem>,
    pub style: TextStyle,
}

impl From<List> for RList<'static> {
    fn from(l: List) -> Self {
        let items: Vec<RListItem<'static>> = l.items.into_iter().map(Into::into).collect();
        RList::new(items).style(l.style)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_item_roundtrip() {
        let item = ListItem::new(
            "entry",
            TextStyle {
                fg: Some(5),
                bg: None,
                modifier: super::super::style::Modifier::NONE,
            },
        );
        let r: RListItem = item.into();
        // no public accessor for text, but roundtrip should not panic
        let _ = r;
    }

    #[test]
    fn list_builds() {
        let l = List {
            items: vec![
                ListItem::new("a", TextStyle::default()),
                ListItem::new("b", TextStyle::default()),
            ],
            style: TextStyle::default(),
        };
        let _: RList = l.into();
    }
}
